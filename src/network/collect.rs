//! Asking every authority what it wants done, and doing it.
//!
//! The far half of [`super::errand`]. This node dials out — over the
//! certificate it enrolled through — because nothing can dial *in* to a
//! private node, which is the reason private nodes exist.
//!
//! ## Obeying is local
//!
//! A `host` errand does not carry a job. It carries a description, and
//! this node writes its own project, its own service row and its own
//! deploy job from it. `deploy` still talks to this node's containerd.
//! Nothing about the queue is distributed.
//!
//! ## Every step is convergent
//!
//! An errand is handed over again whenever the answer did not get back,
//! so carrying one out twice has to mean the same as once: the project
//! is created only if absent, the service is updated rather than
//! duplicated, and the deploy is the same deploy the console queues.

use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use super::errand::{self, Errand, Kind};
use crate::config::Config;
use crate::platform::{projects, registry_credentials, services, slugify};
use crate::runtime::images::Credential;

/// How often a node asks. Short enough that a deployment somebody
/// queued does not feel queued for ever, long enough that a hundred
/// nodes are not a hundred requests a second — and it is a floor on
/// latency rather than a cost, because an authority with nothing to say
/// answers an empty list.
const EVERY: std::time::Duration = std::time::Duration::from_secs(15);

/// The loop, for the runner to hold beside the others.
///
/// A service that returns ends the process, so this waits on the cancel
/// token rather than falling out of the loop — the same shape as the
/// certificate loop next to it.
pub async fn loop_forever(
    database: std::sync::Arc<SqliteDatabase>,
    config: Config,
    container: Container,
    cancel: wabot::lifecycle::Cancel,
) -> anyhow::Result<()> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(EVERY) => {}
        }
        let settled = run_once(&database, &config, &container).await;
        if settled > 0 {
            tracing::info!(settled, "errands carried out");
        }
    }
}

/// Ask every authority once, and carry out what comes back.
///
/// Returns how many errands were settled. Never fails as a whole: one
/// authority that cannot be reached must not stop the others, and one
/// errand that fails is *reported* rather than thrown — the failure
/// belongs on the authority's record of it.
pub async fn run_once(database: &SqliteDatabase, config: &Config, container: &Container) -> usize {
    let authorities = match super::authorities(database).await {
        Ok(authorities) => authorities,
        Err(error) => {
            tracing::warn!(%error, "could not read this node's authorities");
            return 0;
        }
    };

    let mut settled = 0;
    for authority in authorities.iter().filter(|a| a.live()) {
        settled += from_one(database, config, container, &authority.node_id).await;
    }
    settled
}

async fn from_one(
    database: &SqliteDatabase,
    config: &Config,
    container: &Container,
    node_id: &str,
) -> usize {
    let Some(secret) = super::credential_for(database, node_id).await else {
        // A node that joined before errands existed. Re-joining is what
        // fixes it, and saying so once a minute would be noise — the
        // console and `doctor` are where it belongs.
        tracing::debug!(authority = %node_id, "no credential for this authority");
        return 0;
    };
    let endpoint = match super::find(database, node_id).await {
        Ok(Some(node)) => node.endpoint,
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%error, "could not read an authority's row");
            return 0;
        }
    };
    let Some(endpoint) = endpoint else {
        return 0;
    };

    let waiting = match super::call::errands(&endpoint, &secret).await {
        Ok(waiting) => waiting,
        Err(error) => {
            // Expected often enough not to be a warning: an authority
            // is a machine somewhere else, and this runs on a timer.
            tracing::debug!(%error, authority = %node_id, "could not ask for errands");
            return 0;
        }
    };

    let mut settled = 0;
    for order in waiting {
        let id = order.id.clone();
        let outcome = carry_out(database, config, container, node_id, order).await;
        if let Err(reason) = &outcome {
            tracing::warn!(errand = %id, %reason, "could not carry out an errand");
        }

        // Reported even when it failed. An errand nobody answered stays
        // pending for ever on the other node, which is the one state
        // this must not produce.
        match super::call::settle(&endpoint, &secret, &id, outcome.err()).await {
            Ok(()) => settled += 1,
            Err(error) => tracing::warn!(%error, errand = %id, "could not report an errand"),
        }
    }
    settled
}

/// Do what one errand says, or say why not.
async fn carry_out(
    database: &SqliteDatabase,
    config: &Config,
    container: &Container,
    authority: &str,
    order: Errand,
) -> Result<(), String> {
    match order.kind {
        Kind::Host => {
            let host: errand::Host = serde_json::from_value(order.payload)
                .map_err(|error| format!("that is not a host errand: {error}"))?;
            host_service(database, config, container, authority, host).await
        }
        // An instruction from a newer node. Refused with the reason
        // rather than dropped — the authority learns that this node is
        // the old one, which is something somebody can act on.
        Kind::Unknown => {
            Err("this node is older than that instruction and does not know it".into())
        }
    }
}

/// Run a service here, on this node's own rows.
async fn host_service(
    database: &SqliteDatabase,
    config: &Config,
    container: &Container,
    authority: &str,
    host: errand::Host,
) -> Result<(), String> {
    // The credential first: without it the deploy fails at the pull,
    // and the pull is the last thing to run.
    registry_credentials::set(
        database,
        &host.registry,
        &Credential {
            username: host.username.clone(),
            secret: host.secret.clone(),
        },
    )
    .await
    .map_err(|error| error.to_string())?;

    let project = ensure_project(database, &host.project, authority).await?;

    // Looked up by slug, which is what the row is keyed on — two names
    // that differ only in punctuation are one service, and creating a
    // second would be this errand's retry making a mess.
    let existing = services::in_project(database, &project.id, &slugify(&host.service))
        .await
        .map_err(|error| error.to_string())?;

    let service = match existing {
        // Convergent: the same errand twice points one service at the
        // image rather than making a second one beside it.
        Some(service) => {
            services::set_image(database, &service.id, &host.image)
                .await
                .map_err(|error| error.to_string())?;
            services::set_env(database, &service.id, &host.env)
                .await
                .map_err(|error| error.to_string())?;
            service
        }
        None => {
            let env: Vec<(String, String)> = host.env.clone().into_iter().collect();
            let made = services::create(database, &project.id, &host.service, &host.image, &env)
                .await
                .map_err(|error| error.to_string())?;
            // Marked before anything else touches it. A service that
            // existed for even a moment without its origin is one this
            // node's console would have offered to edit.
            services::set_origin(database, &made.id, authority)
                .await
                .map_err(|error| error.to_string())?;
            made
        }
    };

    let _ = config;
    // Its own job, on its own queue. This is the whole of "deploying is
    // local": the authority asked, and what runs is this node's own
    // deployment, indistinguishable from one somebody clicked here.
    let command = crate::deploy::jobs::DeployService {
        service_id: service.id.clone(),
        release_id: None,
    };
    wabot::async_jobs::run_command(container, &command)
        .await
        .map_err(|error| format!("could not queue the deployment: {error}"))?;

    tracing::info!(service = %service.id, image = %host.image, "an errand queued a deployment");
    Ok(())
}

/// The project named by the errand, made if this node has never heard
/// of it.
async fn ensure_project(
    database: &SqliteDatabase,
    name: &str,
    authority: &str,
) -> Result<projects::Project, String> {
    let existing = projects::all(database)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|project| project.name == name);

    // A project this node made itself, with a name an errand happens to
    // match, is **not** the errand's to use. Refused rather than
    // adopted: quietly marking somebody's own project as foreign would
    // take their own work away from them, and it is exactly the kind of
    // thing nobody would look for afterwards.
    if let Some(project) = existing {
        return match project.origin_node_id.as_deref() {
            Some(origin) if origin == authority => Ok(project),
            None => Err(format!(
                "this node already has a project called {name:?} of its own"
            )),
            Some(other) => Err(format!(
                "this node already has a project called {name:?}, from {other}"
            )),
        };
    }

    let project = projects::create(database, name)
        .await
        .map_err(|error| error.to_string())?;
    projects::set_origin(database, &project.id, authority)
        .await
        .map_err(|error| error.to_string())?;
    // Read back, so what is returned carries the origin it was just
    // given rather than the `None` `create` handed out.
    projects::find(database, &project.id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "the project vanished as it was made".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An instruction this version does not know is refused with a
    /// reason, not dropped. The authority learns that this node is the
    /// old one, which is something somebody can act on — an errand that
    /// vanishes is not.
    #[tokio::test]
    async fn an_instruction_from_the_future_is_refused_with_a_reason() {
        let database = crate::db::open_in_memory().await.expect("open");
        let container = Container::new();

        let error = carry_out(
            &database,
            &Config::default(),
            &container,
            "nd-authority",
            Errand {
                id: "er-1".into(),
                kind: Kind::Unknown,
                payload: serde_json::json!({}),
            },
        )
        .await
        .expect_err("refused");

        assert!(error.contains("older"), "{error}");
    }

    /// A payload that will not parse is a refusal with a reason too,
    /// and not a panic on the node that received it.
    #[tokio::test]
    async fn a_host_errand_that_makes_no_sense_is_refused() {
        let database = crate::db::open_in_memory().await.expect("open");
        let container = Container::new();

        let error = carry_out(
            &database,
            &Config::default(),
            &container,
            "nd-authority",
            Errand {
                id: "er-1".into(),
                kind: Kind::Host,
                payload: serde_json::json!({ "project": "only-this" }),
            },
        )
        .await
        .expect_err("refused");

        assert!(error.contains("not a host errand"), "{error}");
    }

    /// The same errand twice points one service at the image rather
    /// than making a second beside it — which is what a re-handed
    /// errand has to mean, since the far end hands one over again
    /// whenever the answer did not get back.
    #[tokio::test]
    async fn the_same_errand_twice_converges_on_one_service() {
        let database = crate::db::open_in_memory().await.expect("open");

        for image in [
            "hub.example.com/proj/app@sha256:aaa",
            "hub.example.com/proj/app@sha256:bbb",
        ] {
            let host = errand::Host {
                project: "shared".into(),
                service: "web".into(),
                image: image.into(),
                registry: "hub.example.com".into(),
                username: "wabot".into(),
                secret: "a-push-token".into(),
                env: Default::default(),
                port: None,
            };
            // The rows, without the queue — `run_command` wants a
            // container this test has no reason to build, and what is
            // being pinned here is convergence of the rows.
            let project = ensure_project(&database, &host.project, "nd-authority")
                .await
                .expect("project");
            let existing = services::in_project(&database, &project.id, &slugify(&host.service))
                .await
                .expect("services");
            match existing {
                Some(service) => services::set_image(&database, &service.id, &host.image)
                    .await
                    .expect("set image"),
                None => {
                    services::create(&database, &project.id, &host.service, &host.image, &[])
                        .await
                        .expect("create");
                }
            }
        }

        let projects = projects::all(&database).await.expect("projects");
        assert_eq!(projects.len(), 1, "a second project was made");
        let service = services::in_project(&database, &projects[0].id, "web")
            .await
            .expect("services")
            .expect("one service");
        assert_eq!(service.image, "hub.example.com/proj/app@sha256:bbb");
    }

    /// What lands on this node from an errand is derived, and has to
    /// read that way from the moment it exists: a service that was ours
    /// for even a moment is one this console would have offered to
    /// edit.
    #[tokio::test]
    async fn what_arrives_on_an_errand_belongs_to_the_node_that_sent_it() {
        let database = crate::db::open_in_memory().await.expect("open");

        let project = ensure_project(&database, "shared", "nd-authority")
            .await
            .expect("project");
        assert!(!project.is_ours());
        assert_eq!(project.origin_node_id.as_deref(), Some("nd-authority"));

        let made = services::create(&database, &project.id, "web", "hub.example/p/a:1", &[])
            .await
            .expect("service");
        services::set_origin(&database, &made.id, "nd-authority")
            .await
            .expect("origin");

        let stored = services::in_project(&database, &project.id, "web")
            .await
            .expect("query")
            .expect("there");
        assert!(!stored.is_ours());
        assert_eq!(stored.origin_node_id.as_deref(), Some("nd-authority"));
    }

    /// A project this node made itself is not an errand's to use, even
    /// when the names match. Adopting it would take somebody's own work
    /// away from them and mark it foreign, which is the kind of thing
    /// nobody would go looking for afterwards.
    #[tokio::test]
    async fn an_errand_does_not_take_over_a_project_this_node_made() {
        let database = crate::db::open_in_memory().await.expect("open");
        let mine = projects::create(&database, "shared").await.expect("mine");

        let error = ensure_project(&database, "shared", "nd-authority")
            .await
            .expect_err("refused");
        assert!(error.contains("of its own"), "{error}");

        let untouched = projects::find(&database, &mine.id)
            .await
            .expect("query")
            .expect("still here");
        assert!(untouched.is_ours(), "somebody's own project was taken over");
    }

    /// And a project from *another* authority is not this one's either
    /// — two nodes pointing one name at different things is the same
    /// conflict the claim rule refuses, one level down.
    #[tokio::test]
    async fn an_errand_does_not_take_over_another_authoritys_project() {
        let database = crate::db::open_in_memory().await.expect("open");
        ensure_project(&database, "shared", "nd-first")
            .await
            .expect("first");

        let error = ensure_project(&database, "shared", "nd-second")
            .await
            .expect_err("refused");
        assert!(error.contains("nd-first"), "and names who has it: {error}");
    }

    /// The credential has to be stored before the deploy runs, because
    /// the pull is the last thing to happen and the first to need it.
    #[tokio::test]
    async fn the_registry_credential_lands_before_anything_is_pulled() {
        let database = crate::db::open_in_memory().await.expect("open");
        registry_credentials::set(
            &database,
            "hub.example.com",
            &Credential {
                username: "wabot".into(),
                secret: "a-push-token".into(),
            },
        )
        .await
        .expect("set");

        assert_eq!(
            registry_credentials::for_reference(&database, "hub.example.com/proj/app@sha256:aaa")
                .await
                .map(|c| c.secret),
            Some("a-push-token".to_string())
        );
    }
}
