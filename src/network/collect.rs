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
use crate::platform::{ports, projects, registry_credentials, replicas, services, slugify};
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

    // What this node's copies of that node's services are doing, before
    // asking for more work — so an authority queueing something has
    // just heard the truth about what is already there, and a replica
    // evicted here stops being asked for in the same round trip.
    // Always, even holding nothing. This used to skip an empty report
    // as a request with nothing in it, and that stopped being true when
    // the report started carrying where the world can dial this node: a
    // node holding no replicas is exactly the one that has never been
    // chosen as an edge, so it stayed private on the authority for ever
    // and could never be chosen — which is the whole of what the field
    // was added for.
    match report_for(database, node_id).await {
        Ok(report) => {
            if let Err(error) = super::call::report(&endpoint, &secret, &report).await {
                tracing::debug!(%error, authority = %node_id, "could not report");
            }
        }
        Err(error) => tracing::warn!(%error, "could not read what this node is holding"),
    }

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

/// What this node is running on behalf of `authority`.
///
/// Read from the rows rather than from containerd: the deploy path
/// already writes what happened to each replica, and asking the runtime
/// again here would be a second opinion that can disagree with the one
/// the console shows.
async fn report_for(
    database: &SqliteDatabase,
    authority: &str,
) -> Result<super::api::Report, String> {
    let projects = projects::all(database)
        .await
        .map_err(|error| error.to_string())?;
    let mut replicas = Vec::new();

    for project in projects
        .iter()
        .filter(|project| project.origin_node_id.as_deref() == Some(authority))
    {
        for service in services::all(database, Some(&project.id))
            .await
            .map_err(|error| error.to_string())?
        {
            for replica in replicas::of_service(database, &service.id)
                .await
                .map_err(|error| error.to_string())?
                .into_iter()
                .filter(|replica| replica.is_here())
            {
                replicas.push(super::api::ReplicaState {
                    project: project.name.clone(),
                    service: service.name.clone(),
                    slot: replica.slot,
                    address: replica.address.clone(),
                    overlay_port: replica.overlay_port,
                    error: replica.last_error.clone(),
                    evicted: replica.evicted(),
                });
            }
        }
    }

    // This node's own reachability, as it sees it. The authority cannot
    // work it out: everything it knows about this node arrived through
    // a token or over the overlay.
    let endpoint = super::me(database)
        .await
        .map_err(|error| error.to_string())?
        .and_then(|me| me.endpoint);

    Ok(super::api::Report { replicas, endpoint })
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
        Kind::Edge => {
            let edge: errand::Edge = serde_json::from_value(order.payload)
                .map_err(|error| format!("that is not an edge errand: {error}"))?;
            serve_name(database, container, authority, edge).await
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

    // What the container listens on, so this node has something to
    // bind on its overlay address for the edge that will proxy to it.
    //
    // Not published and with no hostname, and neither is an omission:
    // the name belongs to the node that placed this, and answering for
    // it here would be two nodes claiming one hostname. Reaching this
    // copy goes through the port its own node opens on the overlay,
    // which is the only address that is unique across nodes.
    if let Some(container_port) = host.port {
        let declared = ports::of_service(database, &service.id)
            .await
            .map_err(|error| error.to_string())?;
        if !declared
            .iter()
            .any(|port| port.container_port == container_port)
        {
            ports::create(database, &service.id, container_port, false, None)
                .await
                .map_err(|error| error.to_string())?;
        }
    }

    // Exactly the copies it was told to run, in the service's own
    // numbering — and **only** those. A `host` errand is the whole of
    // what this node runs for that service, not an addition to it:
    // that is what lets the other node move a replica away, or bring
    // it home, and have this one find out. Before this, a copy whose
    // slot stopped being named kept running for ever, invisible from
    // both sides.
    replicas::ensure_slots(database, &service.id, &host.slots)
        .await
        .map_err(|error| error.to_string())?;
    stop_unnamed(database, config, &project, &service, &host.slots).await?;

    // Nothing left to run: the other node has taken the whole service
    // off this machine, so the rows go too. Not a tombstone — an
    // eviction leaves one because the *authority* has to be told, and
    // here the authority is the one that asked.
    if host.slots.is_empty() {
        services::delete(database, &service.id)
            .await
            .map_err(|error| error.to_string())?;
        return Ok(());
    }
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

/// Answer for a name here, proxying to the replicas the errand named.
///
/// **The claim comes first.** A name belongs to one authority, and a
/// second claim is refused rather than merged — two nodes pointing one
/// hostname at different backends is not a conflict anything can
/// resolve, and choosing silently would make the wrong backend look
/// like the right one. The refusal names who holds it, which is the
/// only outcome somebody can act on. This is what the `claim` table
/// from phase 0 was written for.
///
/// Re-claiming a name this authority already holds succeeds: an errand
/// sent twice must not fail the second time.
async fn serve_name(
    database: &SqliteDatabase,
    container: &Container,
    authority: &str,
    edge: errand::Edge,
) -> Result<(), String> {
    if edge.hostname.trim().is_empty() {
        return Err("that errand names no hostname".into());
    }

    match crate::network::claim(database, &edge.hostname, Some(authority))
        .await
        .map_err(|error| error.to_string())?
    {
        Ok(()) => {}
        Err(refused) => return Err(refused.to_string()),
    }

    let upstreams: Vec<std::net::SocketAddr> = edge
        .upstreams
        .iter()
        .filter_map(|address| match address.parse() {
            Ok(parsed) => Some(parsed),
            Err(_) => {
                tracing::warn!(%address, "upstream skipped: not an address");
                None
            }
        })
        .collect();

    // Nothing to send it to is not a route. Answering 404 is truthful;
    // a name pointing at an empty list is a hung request waiting to
    // happen.
    if upstreams.is_empty() {
        crate::edge::routes::forget_control_plane(database, &edge.hostname)
            .await
            .map_err(|error| error.to_string())?;
        crate::network::release(database, &edge.hostname)
            .await
            .map_err(|error| error.to_string())?;
        reload_routes(container).await;
        return Ok(());
    }

    // `None` for the service: there is no local service behind this
    // name, and that absence is exactly what stops the local sync from
    // pruning the row — see `routes::retain_proxies`.
    crate::edge::routes::upsert(
        database,
        &edge.hostname,
        &crate::edge::routes::Upstream::Proxy(upstreams),
        None,
    )
    .await
    .map_err(|error| error.to_string())?;

    reload_routes(container).await;

    tracing::info!(
        hostname = %edge.hostname,
        upstreams = edge.upstreams.len(),
        %authority,
        "serving a name for another node"
    );
    Ok(())
}

/// Put the table the listener reads in step with the rows.
///
/// Writing the route row is not serving the name: the edge answers from
/// a table held in memory, and until something reloads it the node keeps
/// proxying to whatever it had. It took a restart to see that — the row
/// was right the whole time and the requests were going somewhere else
/// entirely.
///
/// The deployer out of the container, not a new one: a `Deployer::new`
/// has no handle on the live table, so it would write the database again
/// and change nothing. A node with no deployer registered has no
/// listener either, which is every test in this file.
async fn reload_routes(container: &Container) {
    match container.try_resolve::<crate::deploy::Deployer>() {
        Ok(deployer) => deployer.sync_routes().await,
        Err(_) => tracing::debug!("no deployer here; the routes stay as they are"),
    }
}

/// Stop every copy of this service here that the errand did not name.
///
/// The rows go with the containers. A replica this node is no longer
/// asked to run is not evicted — nobody threw it out, the node that
/// placed it changed its mind — so there is nothing to report and
/// nothing to keep.
async fn stop_unnamed(
    database: &SqliteDatabase,
    config: &Config,
    project: &projects::Project,
    service: &services::Service,
    slots: &[u32],
) -> Result<(), String> {
    let deployer = crate::deploy::Deployer::new(std::sync::Arc::new(database.clone()), config);

    for replica in replicas::of_service(database, &service.id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|replica| replica.is_here() && !slots.contains(&replica.slot))
    {
        if let Err(error) = deployer.stop_replica(project, service, &replica).await {
            // Reported and carried on from: the row is going either
            // way, and a container this node could not reach must not
            // keep it listed as something it runs.
            tracing::warn!(slot = replica.slot, %error, "stopping a copy that was taken away");
        }
        replicas::remove(database, &replica.id)
            .await
            .map_err(|error| error.to_string())?;
    }
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

    /// A copy placed on another node is reached through a port on that
    /// node's overlay address, and that port can only be opened if the
    /// node knows what the container listens on. The errand carried the
    /// field and nothing filled it, so the copy ran and nothing could
    /// reach it — which is the whole point of placing it.
    ///
    /// No hostname and not published, deliberately: the name belongs to
    /// the node that placed this, and answering for it here would be
    /// two nodes claiming one.
    #[tokio::test]
    async fn a_hosted_copy_knows_what_its_container_listens_on() {
        let database = crate::db::open_in_memory().await.expect("open");
        let container = Container::new();

        let order = Errand {
            id: "er-one".into(),
            kind: Kind::Host,
            payload: serde_json::json!({
                "project": "shared",
                "service": "web",
                "image": "registry.example/web@sha256:abc",
                "registry": "registry.example",
                "username": "errand",
                "secret": "s3cret",
                "port": 80,
                "slots": [3],
            }),
        };
        // The deployment cannot run — there is no containerd here — but
        // the rows it is queued from are written before that.
        let _ = carry_out(&database, &Config::default(), &container, "nd-a", order).await;

        let service = services::all(&database, None)
            .await
            .expect("services")
            .pop()
            .expect("the errand made one");
        let declared = ports::of_service(&database, &service.id)
            .await
            .expect("ports")
            .pop()
            .expect("it knows what to bind");

        assert_eq!(declared.container_port, 80);
        assert!(declared.hostname.is_none(), "the name is not this node's");
        assert!(declared.host_port.is_none(), "nothing published it here");
    }

    /// A node holding nothing still has something to say: where the
    /// world can dial it. The report used to be skipped when it carried
    /// no replicas, which was true of the only thing in it at the time
    /// — and it meant a node that had never been given work stayed
    /// private on its authority for ever, so it could never be chosen
    /// as an edge, which is the one thing that field exists for.
    #[tokio::test]
    async fn a_node_holding_nothing_still_says_where_it_can_be_reached() {
        let database = crate::db::open_in_memory().await.expect("open");
        crate::node::settings::set_domain(&database, Some("alpine.example"))
            .await
            .expect("domain");
        crate::network::ensure_self(&database, &Config::default())
            .await
            .expect("self");

        let report = report_for(&database, "nd-authority").await.expect("report");

        assert!(report.replicas.is_empty(), "nothing was placed here");
        assert_eq!(report.endpoint.as_deref(), Some("alpine.example:443"));
    }

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

    /// A name belongs to one authority, and this is where phase 0's
    /// claim rule finally does its job: two nodes pointing one hostname
    /// at different backends is not a conflict anything can resolve,
    /// and choosing silently would make the wrong backend look right.
    #[tokio::test]
    async fn a_name_is_served_for_one_authority_and_the_second_is_told_who_has_it() {
        let database = crate::db::open_in_memory().await.expect("open");
        let edge = errand::Edge {
            hostname: "app.example.com".into(),
            upstreams: vec!["10.42.0.3:30000".into(), "10.42.0.3:30001".into()],
        };

        serve_name(&database, &Container::new(), "nd-first", edge.clone())
            .await
            .expect("served");

        // The route is there, with one upstream per replica.
        let routes = crate::edge::routes::load_all(&database)
            .await
            .expect("routes");
        let (_, upstream) = routes
            .iter()
            .find(|(host, _)| host == "app.example.com")
            .expect("a route");
        assert_eq!(
            *upstream,
            crate::edge::routes::Upstream::Proxy(vec![
                "10.42.0.3:30000".parse().unwrap(),
                "10.42.0.3:30001".parse().unwrap(),
            ])
        );

        // Sent twice by the same authority is the same instruction.
        serve_name(&database, &Container::new(), "nd-first", edge.clone())
            .await
            .expect("again");

        let refused = serve_name(&database, &Container::new(), "nd-second", edge)
            .await
            .expect_err("refused");
        assert!(
            refused.contains("nd-first"),
            "and names who has it: {refused}"
        );
    }

    /// A route this node keeps for somebody else must survive its own
    /// deployments. It has no local service behind it, and that absence
    /// is what the local sync uses to leave it alone — without it, the
    /// name went off the air every time anything deployed here.
    #[tokio::test]
    async fn an_edge_route_survives_a_local_sync() {
        let database = crate::db::open_in_memory().await.expect("open");
        serve_name(
            &database,
            &Container::new(),
            "nd-first",
            errand::Edge {
                hostname: "app.example.com".into(),
                upstreams: vec!["10.42.0.3:30000".into()],
            },
        )
        .await
        .expect("served");

        // What a local deployment does at the end of every sync.
        crate::edge::routes::retain_proxies(&database, &[])
            .await
            .expect("retain");

        let routes = crate::edge::routes::load_all(&database)
            .await
            .expect("routes");
        assert!(
            routes.iter().any(|(host, _)| host == "app.example.com"),
            "a local deploy took somebody else's name off the air"
        );
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
                slots: vec![1],
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
