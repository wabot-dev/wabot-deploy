//! What this machine's settings page posts to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use super::auth::{signed_in, SessionMiddleware};
use super::ConsoleState;
use crate::views::pages::node_user_node_settings_page::NodeTab;

/// The server half.
#[injectable]
pub struct NodeRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl NodeRestController {
    /// The node's certificate authority, as a file.
    ///
    /// Because a client that cannot verify is a client using `require`,
    /// which is encryption without identity. A container is handed this at
    /// `/etc/wabot/ca.crt`; anything else — a laptop, a CI runner, a
    /// machine elsewhere — has nowhere to get it from, and telling somebody
    /// to `psql` their way into a container to copy a file is not an
    /// answer.
    ///
    /// It is the authority's **public** half, which is what an authority is
    /// for. Behind a session anyway: what it discloses is that this node
    /// exists and signs things, and that is nobody's business but the
    /// operator's.
    #[get("/ca.crt")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn authority(&self, _request: Request) -> RestResult<Response> {
        if signed_in(&self.auth).is_none() {
            return Ok(Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Body::empty())
                .expect("a constant response is well-formed"));
        }
        let pem = match crate::edge::certs::ca_certificate_pem(&self.state.database).await {
            Ok(pem) => pem,
            Err(error) => {
                tracing::warn!(%error, "could not read this node's authority");
                return Ok(Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Body::empty())
                    .expect("a constant response is well-formed"));
            }
        };

        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "application/x-pem-file")
            // Named, because a file called `ca.crt` in a downloads folder
            // beside four others is a file nobody can place.
            .header(
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"wabot-node-ca.crt\"",
            )
            .body(Body::from(pem))
            .expect("a constant response is well-formed"))
    }

    #[post("/node/certificate")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_certificate(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        // **Back to the form.** This answered to `self_path` — the
        // node's page in the *network* list — which has no error note
        // and no `checked` parameter, so a refusal and a success were
        // both dropped: the page jumped somewhere else and said
        // nothing. `node-0.tobaw.shop` was refused correctly, for not
        // resolving, and there was nowhere for it to say so. The form
        // has been on the Domain tab since the node's settings grew
        // tabs; the redirect never followed it.
        let here = &NodeTab::Domain.path();

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        // "Try again" is the same form with nothing changed. It exists
        // because the usual fix for a refusal is somewhere else — a DNS
        // record, a firewall — and making somebody re-type the domain to
        // retry reads as though the domain were what was wrong.
        if form.contains_key("retry") {
            self.state.certificates.now();
            return Ok(super::auth::see_other(here));
        }

        let settled = crate::usecases::node_user_set_node_domain::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
            super::auth::field(&form, "domain"),
        )
        .await;

        // The loop owns issuance — both the local certificate carrying
        // the new name and the public one. Asking it here is what makes
        // saving a domain reissue rather than merely record.
        match settled {
            Ok(crate::dtos::responses::domain_settled::Settled::Named(domain)) => {
                self.state.certificates.now();
                Ok(super::auth::see_other(&format!(
                    "{here}?{}",
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("checked", &format!("{domain} resolves here."))
                        .finish()
                )))
            }
            Ok(crate::dtos::responses::domain_settled::Settled::Cleared) => {
                self.state.certificates.now();
                Ok(super::auth::back_with_error(
                    here,
                    "the domain was cleared — this node serves its own certificate",
                ))
            }
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Say what this node is willing to do, for anybody at all.
    ///
    /// Not "for these other nodes" — that is a grant, and it is decided
    /// when two nodes agree to know each other. This is one layer above
    /// and it binds this node too: a machine small enough to prefer
    /// running nothing places every copy of its own services elsewhere,
    /// and a machine with a perfectly good address can decline to answer
    /// for names.
    ///
    /// Turning `edge` off makes this node **private**, which is not a
    /// separate switch and must not become one: private has never meant
    /// anything except "does not offer to answer for names".
    /// What this machine keeps for itself.
    ///
    /// **The floor is a measurement, not a constant.** A reserve under
    /// what this node's own processes are using right now — this binary,
    /// containerd, the shims — is refused with the reading, because the
    /// node can answer that question about itself and 256 MB cannot. The
    /// old constant was chosen for exactly this reason and had to guess.
    ///
    /// And a reserve that leaves less than is already promised is refused
    /// too, naming the figure: the ceilings on existing services are
    /// reservations, so accepting it would make every "does this fit"
    /// answer on the node wrong until somebody deleted something.
    #[post("/node/reserves")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_reserves(&self, request: Request) -> RestResult<Response> {
        // The tab this form is on, not `/node` — which is a redirect to
        // the first tab, so a refused reserve carried its message to the
        // domain page and a saved one landed two tabs from the number it
        // had just changed.
        let here = &NodeTab::Resources.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        if let Err(reason) = crate::usecases::node_user_set_reserves::run(
            &self.state.database,
            &self.state.deployer,
            &account,
            super::auth::field(&form, "memory"),
            super::auth::field(&form, "cpu"),
        )
        .await
        {
            return Ok(super::auth::back_with_error(here, &reason));
        }

        Ok(super::auth::see_other(&format!("{here}#reserves")))
    }

    #[post("/node/capabilities")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_capabilities(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Allows.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        // The one POST under `/node` that had no admin check, and the
        // omission was not small. `evict_ungranted` and
        // `release_ungranted` run at boot and act on exactly these
        // values, so turning `host` off here throws off every replica
        // this node holds for anybody, and turning `edge` off releases
        // every name it claims — from any account with a session.
        //
        // The page that draws the form is behind `is_admin` already, so
        // this was hidden and not enforced, which is the distinction
        // every island in this console is written around.
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked = crate::dtos::requests::set_capabilities::SetCapabilities {
            provides: super::auth::capabilities_ticked(&form, ""),
            owns: form.contains_key("own"),
        };
        if let Err(reason) = crate::usecases::node_user_set_capabilities::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
            &asked,
        )
        .await
        {
            return Ok(super::auth::back_with_error(here, &reason));
        }

        Ok(super::auth::see_other(here))
    }

    /// Redeploy every managed engine here, because something they read
    /// at startup has changed.
    ///
    /// Queued rather than done: a deployment is this node's own job,
    /// which is what "obeying is local" has meant since phase 3, and a
    /// form that waited for three databases to restart is a form that
    /// times out.
    async fn redeploy_databases(&self) {
        let Ok(services) = crate::repositories::services::all(&self.state.database, None).await
        else {
            return;
        };
        for service in services.iter().filter(|service| service.kind.is_managed()) {
            let command = crate::deploy::jobs::DeployService {
                service_id: service.id.clone(),
                release_id: None,
            };
            if let Err(error) =
                wabot::async_jobs::run_command(&self.state.container, &command).await
            {
                tracing::error!(service = %service.slug, %error, "could not queue it");
            }
        }
    }

    /// What this node backs itself up to, and when.
    ///
    /// **The destination is checked here and not only when the run comes
    /// round.** A schedule saved against `sftp://…` — one letter from a
    /// scheme this understands — is a job that fails at three in the
    /// morning for a reason that was available the moment somebody typed
    /// it. Same rule as the certificate form, which reads a pair of files
    /// before it accepts them.
    #[post("/node/backups")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_backups(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked = crate::dtos::requests::set_backup_plan::SetBackupPlan {
            destination: super::auth::field(&form, "destination").to_string(),
            cadence: super::auth::field(&form, "cadence").to_string(),
            hour: super::auth::field(&form, "hour").to_string(),
            weekday: super::auth::field(&form, "weekday").to_string(),
            keep_days: super::auth::field(&form, "keep_days").to_string(),
            // Presence, not value: an unticked checkbox sends nothing at
            // all, so this is the last place that can tell it from a
            // form that never carried the field.
            archiving: form.contains_key("archiving"),
        };

        let saved = crate::usecases::node_user_set_backup_plan::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await;

        match saved {
            Ok(saved) => {
                // Something the managed engines read at startup moved, so
                // they have to come back with it.
                if saved.archiving_changed {
                    self.redeploy_databases().await;
                }
                Ok(super::auth::see_other(here))
            }
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Store the bucket's credential — in the file, never in the
    /// database.
    ///
    /// Jorge asked for the mechanism: a bucket used to need an SSH
    /// session and a text editor. What did not move is *where* it goes,
    /// and the reasoning is on `config::BackupConfig`: a key that can
    /// read every backup in the network must not be inside the thing
    /// being backed up.
    ///
    /// **Asked of the provider before it is written**, when the
    /// destination is a bucket. The call is a listing, which is what a
    /// backup starts with anyway — so a credential that saves is one that
    /// works, and a typo is refused here rather than at three in the
    /// morning by a schedule nobody is watching.
    #[post("/node/backups/credentials")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_bucket_credentials(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = crate::dtos::requests::set_bucket_credentials::SetBucketCredentials {
            access_key_id: super::auth::field(&form, "access_key_id").to_string(),
            secret_access_key: super::auth::field(&form, "secret_access_key").to_string(),
            region: super::auth::field(&form, "region").to_string(),
            endpoint: super::auth::field(&form, "endpoint").to_string(),
        };
        match crate::usecases::node_user_set_bucket_credentials::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await
        {
            Ok(()) => Ok(super::auth::see_other(here)),
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Take the credential out of the file.
    #[post("/node/backups/credentials/forget")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn forget_bucket_credentials(&self, _request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        if let Err(reason) = crate::usecases::node_user_forget_bucket_credentials::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            return Ok(super::auth::back_with_error(here, &reason));
        }
        Ok(super::auth::see_other(here))
    }

    /// Take one now.
    ///
    /// Started and not awaited: a backup is minutes of `pg_basebackup`
    /// and file copying, and a browser waiting on that times out
    /// somewhere in the middle — leaving the operator with no page and a
    /// backup still running. The page it comes back to says one is
    /// running, and how the last one went.
    #[post("/node/backups/now")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn back_up_now(&self, _request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        // Refused rather than queued. Two of these is two copies of every
        // database at once on a machine with one core, and a second click
        // on a button that has not answered yet is the ordinary way to
        // ask for that.
        match crate::usecases::node_user_back_up_now::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            Ok(()) => Ok(super::auth::see_other(here)),
            Err(reason) => Ok(super::auth::back_with_error(here, &reason)),
        }
    }

    /// Choose where this name's certificate comes from.
    ///
    /// The file pair is read and checked *here*, before the choice is
    /// stored. Storing a policy that cannot work would make the
    /// renewal loop fail on every pass with the reason in the journal,
    /// and this console exists so that reasons do not live there. The
    /// worse case is the one this refuses outright: a certificate and
    /// key that do not match install cleanly and then break every
    /// handshake, including the one serving this page.
    #[post("/node/certificate/source")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_certificate_source(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(super::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(super::auth::see_other("/"));
        }
        // **Back to the form.** This answered to `self_path` — the
        // node's page in the *network* list — which has no error note
        // and no `checked` parameter, so a refusal and a success were
        // both dropped: the page jumped somewhere else and said
        // nothing. `node-0.tobaw.shop` was refused correctly, for not
        // resolving, and there was nowhere for it to say so. The form
        // has been on the Domain tab since the node's settings grew
        // tabs; the redirect never followed it.
        let here = &NodeTab::Domain.path();

        let form = match super::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // The node's own name, or the fallback while it has none — the
        // same key the renewal loop will look this up under.
        let name = match crate::usecases::node_user_set_certificate_source::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            Ok(name) => name,
            Err(reason) => return Ok(super::auth::back_with_error(here, &reason)),
        };

        let renew_with = match super::auth::source_from(&form, &name) {
            Ok(renew_with) => renew_with,
            Err(reason) => return Ok(super::auth::back_with_error(here, &reason)),
        };

        if let Err(error) = crate::commands::store_certificate_source::run(
            &self.state.database,
            &self.state.config,
            &name,
            &renew_with,
        )
        .await
        {
            return Ok(super::auth::back_with_error(here, &error));
        }
        // The loop installs it. Same reason as saving a domain: this
        // console asks, it does not issue.
        self.state.certificates.now();

        Ok(super::auth::see_other(here))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::config::Config;
    use crate::console::tests::Console;
    use crate::network;
    use crate::views::components::node_certificate::CertificateState;
    use wabot::rest::axum::http::StatusCode;

    /// Empty means the default, and a number survives the round trip.
    #[tokio::test]
    async fn a_reserve_is_stored_and_cleared_from_the_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($memory:expr) => {{
                let response = console
                    .harness
                    .post("/node/reserves")
                    .header("cookie", &cookie)
                    .form(&[("memory", $memory), ("cpu", "")])
                    .send()
                    .await;
                response.header("location").unwrap_or_default().to_string()
            }};
        }

        let saved = save!("512 MB");
        // Back to the tab the form is on. It was `/node`, which is a
        // redirect to the first tab — so saving a reserve landed on the
        // domain page and the fragment pointed at a card that was not
        // there.
        assert!(saved.ends_with("/node/resources#reserves"), "{saved}");
        assert_eq!(
            crate::node::settings::memory_reserve(&console.database).await,
            Some(512 * 1024 * 1024)
        );

        // Empty is "whatever this machine's size implies", not nought.
        save!("");
        assert_eq!(
            crate::node::settings::memory_reserve(&console.database).await,
            None,
            "empty is the default, not zero"
        );

        // And a size the parser refuses never reaches the row.
        let refused = save!("lots");
        assert!(refused.contains("error="), "{refused}");
    }

    /// Every capability has a checkbox, because the handler writes a
    /// setting for every one of them.
    ///
    /// **The form used to draw two of three.** So saving it turned
    /// `store` off in silence: the node stopped being offered as
    /// somewhere a standby could go, and refused the errands for the ones
    /// it already held. The card is generated from `Capability::ALL` now
    /// and the words come out of an exhaustive `match`, which makes the
    /// mistake a compile error — this is the belt.
    #[tokio::test]
    async fn every_capability_has_a_checkbox_on_the_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let body = console
            .harness
            .get(&NodeTab::Allows.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        for capability in network::capability::Capability::ALL {
            assert!(
                body.contains(&format!("name=\"{}\"", capability.name())),
                "{} has no checkbox, and saving this form would turn it off: {body}",
                capability.name()
            );
        }
        assert!(
            body.contains("name=\"own\""),
            "and the switch that is not a capability: {body}"
        );
    }

    /// What this node agrees to do for the whole network is not a
    /// member's decision.
    ///
    /// **This guard was missing**, and it was the only POST under `/node`
    /// without one — every other form in this controller asks. What it
    /// cost is not a setting written wrongly: `evict_ungranted` and
    /// `release_ungranted` run at boot and act on these values, so a
    /// member turning `host` off threw off every replica this node holds
    /// for anybody, and turning `edge` off released every name it claims.
    /// The form was hidden behind `is_admin` and the handler was not,
    /// which is the difference this console is written around, and the
    /// tests around it all signed in as the administrator.
    #[tokio::test]
    async fn only_an_admin_may_change_what_this_node_provides() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        let response = console
            .harness
            .post("/node/capabilities")
            .header("cookie", &member)
            // Every box unticked, which is how this form says "off".
            .form::<&str, &str>(&[])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));

        for capability in network::capability::Capability::ALL {
            assert!(
                network::capability::provides(&console.database, capability).await,
                "a member turned {} off for the whole node",
                capability.name()
            );
        }
        assert!(
            crate::network::capability::owns(&console.database).await,
            "and stopped it owning anything"
        );
    }

    /// A node cannot stop being an owner while it still owns something.
    ///
    /// Its projects would be in a state nothing can act on: not editable,
    /// because the node no longer originates work; not evictable, because
    /// they are its own and no other node placed them. Like a foreign
    /// key, and the message names the count so it is a thing somebody can
    /// go and do.
    #[tokio::test]
    async fn a_node_that_owns_projects_cannot_stop_being_an_owner() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            () => {
                console
                    .harness
                    .post("/node/capabilities")
                    .header("cookie", &cookie)
                    .form(&[("host", "1"), ("edge", "1")])
                    .send()
                    .await
                    .header("location")
                    .unwrap_or_default()
                    .to_string()
            };
        }

        crate::repositories::projects::create(&console.database, "mine")
            .await
            .expect("a project of its own");

        let refused = save!();
        assert!(refused.contains("error="), "{refused}");
        assert!(
            crate::network::capability::owns(&console.database).await,
            "and nothing was written"
        );

        // With nothing of its own left, the same click is accepted.
        let projects = crate::repositories::projects::all(&console.database)
            .await
            .expect("projects");
        for project in projects {
            crate::repositories::projects::delete(&console.database, &project.id)
                .await
                .expect("removed");
        }
        let saved = save!();
        assert!(!saved.contains("error="), "{saved}");
        assert!(!crate::network::capability::owns(&console.database).await);
    }

    /// This node's settings are tabs, like a service's.
    ///
    /// Three unrelated cards in one column, and Jorge asked for the same
    /// shape he had asked for on a service. One URL each, so a tab can be
    /// linked and read with scripting off, and the page loads only what it
    /// shows — the domain tab is the only one that reads a certificate off
    /// disk.
    #[tokio::test]
    async fn the_nodes_settings_are_tabs() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! get {
            ($path:expr) => {
                console
                    .harness
                    .get($path)
                    .header("cookie", &cookie)
                    .send()
                    .await
            };
        }

        // The bare path names the first tab, and so does a word that is
        // not one — a hand-typed URL, or a link from before there were
        // tabs at all.
        assert_eq!(get!("/node").header("location"), Some("/node/domain"));
        assert_eq!(
            get!("/node/nonsense").header("location"),
            Some("/node/domain")
        );

        // Each card on exactly one tab. A card on two is a card somebody
        // saves twice.
        let mut seen = Vec::new();
        for tab in NodeTab::all() {
            let body = get!(&tab.path()).body;
            assert!(
                body.contains(r#"aria-current="page""#),
                "the {} tab does not say it is current: {body}",
                tab.slug()
            );
            for (form, name) in [
                ("/node/certificate", "certificate"),
                ("/node/capabilities", "capabilities"),
                ("/node/reserves", "reserves"),
                ("/node/backups", "backups"),
            ] {
                if body.contains(form) {
                    seen.push((name, tab.slug()));
                }
            }
        }
        assert_eq!(
            seen,
            vec![
                ("certificate", "domain"),
                ("capabilities", "allows"),
                ("reserves", "resources"),
                // The tab is `backup` and what it submits to is
                // `backups`: a GET view and a POST can share a path, but
                // a form whose action is the page it sits on reads as a
                // coincidence rather than a decision.
                ("backups", "backup"),
            ],
            "a form is missing, or on more than one tab"
        );
    }

    /// The tab Jorge asked for, and what had to move onto it.
    ///
    /// The write-ahead log switch was on Capabilities, as a third thing
    /// the node either does or does not do. It belongs beside the
    /// schedule: point-in-time recovery is how far a restore *reaches*
    /// from a backup, and with no backup to anchor it the log is disk
    /// nothing can use.
    #[tokio::test]
    async fn the_backup_tab_carries_the_plan_and_the_log_switch() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(body.contains(r#"action="/node/backups""#), "{body}");
        assert!(
            body.contains(r#"name="archiving""#),
            "the log switch is here now: {body}"
        );
        assert!(
            body.contains(r#"action="/node/backups/now""#),
            "and the button that takes one now: {body}"
        );
        // The island hides what does not apply; it does not build the
        // form. Every field is in the HTML the node produced, which is
        // what makes this page work with scripting off.
        assert!(body.contains(r#"data-island="fields""#), "{body}");
        assert!(body.contains(r#"data-when="cadence=weekly""#), "{body}");
        conditions_name_real_controls(&body);

        // And it is gone from where it was. A switch on two tabs is a
        // switch somebody saves twice, with two different answers.
        let allows = console
            .harness
            .get(&NodeTab::Allows.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            !allows.contains(r#"name="archiving""#),
            "the log switch is still on Capabilities: {allows}"
        );
    }

    /// A plan survives the form and comes back selected.
    ///
    /// The second half is the half that has been wrong here before:
    /// `selected` is read by *presence*, so a value rendered as
    /// `selected=(false)` selects — which is how a selector came to name
    /// the wrong project. Every option on this form branches.
    #[tokio::test]
    async fn a_backup_plan_is_stored_and_comes_back_selected() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let saved = console
            .harness
            .post("/node/backups")
            .header("cookie", &cookie)
            .form(&[
                ("destination", "/mnt/backups"),
                ("cadence", "weekly"),
                ("hour", "4"),
                ("weekday", "2"),
                ("keep_days", "30"),
            ])
            .send()
            .await;
        assert_eq!(saved.header("location"), Some("/node/backup"));

        let plan = crate::node::backups::plan(&console.database).await;
        assert_eq!(plan.destination.as_deref(), Some("/mnt/backups"));
        assert_eq!(plan.cadence, crate::node::backups::Cadence::Weekly);
        assert_eq!(plan.hour, 4);
        assert_eq!(plan.weekday, 2);
        assert_eq!(plan.keep_days, 30);

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains(r#"value="weekly" selected"#), "{body}");
        assert!(body.contains(r#"value="4" selected"#), "the hour: {body}");
        assert!(body.contains(r#"value="2" selected"#), "the day: {body}");
        assert!(body.contains(r#"value="30""#), "the window: {body}");
    }

    /// A destination that cannot work is refused when it is typed.
    ///
    /// Not when the run comes round. A schedule saved against
    /// `sftp://…` — one letter from a scheme this understands — is a job
    /// that fails at three in the morning for a reason that was
    /// available the moment somebody typed it.
    #[tokio::test]
    async fn a_destination_this_node_cannot_write_to_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($destination:expr) => {{
                let response = console
                    .harness
                    .post("/node/backups")
                    .header("cookie", &cookie)
                    .form(&[
                        ("destination", $destination),
                        ("cadence", "daily"),
                        ("hour", "3"),
                        ("weekday", "6"),
                        ("keep_days", "7"),
                    ])
                    .send()
                    .await;
                response.header("location").unwrap_or_default().to_string()
            }};
        }

        let refused = save!("sftp://host/backups");
        assert!(refused.contains("error="), "{refused}");
        // A bucket with no credentials on this node: the same answer, and
        // it names the file to put them in rather than the failure.
        let no_credentials = save!("s3://somebodys-bucket/nodes");
        assert!(no_credentials.contains("error="), "{no_credentials}");
        assert!(
            no_credentials.contains("card+below"),
            "and says where they go — the card on this tab, not an SSH session: {no_credentials}"
        );
        assert_eq!(
            crate::node::backups::plan(&console.database).await,
            crate::node::backups::Plan::default(),
            "a refused form must store nothing"
        );

        // And a window of no days, which would delete each backup as
        // soon as it was taken.
        let response = console
            .harness
            .post("/node/backups")
            .header("cookie", &cookie)
            .form(&[
                ("destination", ""),
                ("cadence", "daily"),
                ("hour", "3"),
                ("weekday", "6"),
                ("keep_days", "0"),
            ])
            .send()
            .await;
        let refused = response.header("location").unwrap_or_default().to_string();
        assert!(refused.contains("error="), "{refused}");
    }

    /// The switch reaches the row, and the databases are told.
    ///
    /// The telling is what makes it more than a checkbox: `archive_mode`
    /// is a postmaster setting written into the container's arguments, so
    /// without a redeployment the switch reads on, the page says on, and
    /// nothing archives.
    #[tokio::test]
    async fn the_log_switch_is_saved_from_the_backup_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($fields:expr) => {
                console
                    .harness
                    .post("/node/backups")
                    .header("cookie", &cookie)
                    .form($fields)
                    .send()
                    .await
            };
        }

        // A checkbox arrives only when ticked, so its absence is the
        // answer rather than a missing field.
        save!(&[("cadence", ""), ("keep_days", "7")]);
        assert!(!crate::node::settings::archiving(&console.database).await);

        save!(&[("cadence", ""), ("keep_days", "7"), ("archiving", "1")]);
        assert!(crate::node::settings::archiving(&console.database).await);
    }

    /// What is on the disk, and whose it is.
    ///
    /// A shared backup root holds several nodes' copies — that is what
    /// makes one bucket the answer for a network rather than for a
    /// machine — so a row about another node has to say so rather than
    /// read as this one's.
    #[tokio::test]
    async fn a_backup_on_the_disk_says_whose_it_is() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let write = |node: &str, at: i64, volumes: usize| {
            let into = console
                .data_dir
                .join("backups")
                .join("nodes")
                .join(node)
                .join(at.to_string());
            std::fs::create_dir_all(&into).expect("a backup directory");
            let manifest = serde_json::json!({
                "format": 1,
                "taken_by": "wabot-deploy 0.0.0-test",
                "taken_at": at,
                "node_id": node,
                "node_name": format!("{node}.example"),
                "volumes": (0..volumes).map(|index| serde_json::json!({
                    "container": format!("demo.orders-{index}"),
                    "bytes": 1024,
                    "how": "Consistent",
                })).collect::<Vec<_>>(),
                "images": [],
            });
            std::fs::write(
                into.join("manifest.json"),
                serde_json::to_vec(&manifest).expect("a manifest"),
            )
            .expect("write it");
        };

        let me = crate::network::me(&console.database)
            .await
            .expect("read")
            .expect("this node");
        write(&me.id, 1_700_000_000_000, 2);
        write("some-other-node", 1_600_000_000_000, 1);

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("2 volume(s)"), "{body}");
        assert!(body.contains("this node"), "{body}");
        assert!(
            body.contains("some-other-node.example"),
            "the other node's row names it: {body}"
        );
    }

    /// A bucket is configured from the console, and the secret is not in
    /// the database.
    ///
    /// Jorge asked for the mechanism — configuring one was an SSH session
    /// and a text editor — and the answer is *which file*, not who writes
    /// it: a key that can read every backup in the network must not be
    /// inside the thing being backed up.
    #[tokio::test]
    async fn a_bucket_credential_is_stored_in_the_file_and_not_the_database() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let path = console.data_dir.join("config.toml");

        let saved = console
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", "s3cr3t-from-the-provider"),
                ("region", "us-east-1"),
                ("endpoint", "https://s3.us-west-004.backblazeb2.com"),
            ])
            .send()
            .await;
        assert_eq!(saved.header("location"), Some("/node/backup"));

        let file = std::fs::read_to_string(&path).expect("the config file");
        assert!(file.contains("AKIAEXAMPLE"), "{file}");
        assert!(file.contains("s3cr3t-from-the-provider"), "{file}");

        // And nowhere near the database. Read as bytes because the
        // question is "is that string anywhere in this file", not "is
        // there a row" — a secret that leaked into any column at all
        // would travel in every backup.
        let db = std::fs::read(console.data_dir.join("db").join("node.db")).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&db).contains("s3cr3t-from-the-provider"),
            "the secret must not be in the node's own database"
        );

        // The page shows that there is one, and never what it is.
        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("AKIAEXAMPLE"),
            "the key id is not a secret: {body}"
        );
        assert!(
            !body.contains("s3cr3t-from-the-provider"),
            "the secret is never rendered back"
        );
        assert!(
            body.contains(r#"action="/node/backups/credentials/forget""#),
            "and it can be removed: {body}"
        );
    }

    /// Changing the region does not mean retyping the secret.
    ///
    /// The field renders empty whatever is stored — it is never shown
    /// again — so empty on save has to mean "keep it". Reading it as
    /// "clear it" would wipe the credential of everybody who came to fix
    /// a region, and they would find out at the next backup.
    #[tokio::test]
    async fn an_empty_secret_keeps_the_one_that_is_stored() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($secret:expr, $region:expr) => {
                console
                    .harness
                    .post("/node/backups/credentials")
                    .header("cookie", &cookie)
                    .form(&[
                        ("access_key_id", "AKIAEXAMPLE"),
                        ("secret_access_key", $secret),
                        ("region", $region),
                        ("endpoint", ""),
                    ])
                    .send()
                    .await
            };
        }

        save!("kept-across-the-edit", "us-east-1");
        let moved = save!("", "eu-central-1");
        assert_eq!(moved.header("location"), Some("/node/backup"));

        let stored = console.config.s3().expect("still a credential");
        assert_eq!(stored.secret_access_key, "kept-across-the-edit");
        assert_eq!(stored.region, "eu-central-1");

        // With nothing stored there is nothing to keep, and the refusal
        // says so rather than writing a credential with no secret in it.
        let bare = Console::new().await;
        let bare_cookie = bare.signed_in().await;
        let refused = bare
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &bare_cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", ""),
                ("region", "us-east-1"),
                ("endpoint", ""),
            ])
            .send()
            .await;
        assert!(
            refused
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "{:?}",
            refused.header("location")
        );
    }

    /// Forgetting the credential while the backups go to a bucket is
    /// refused.
    ///
    /// The button is disabled too, but that is a courtesy — the check
    /// that counts is here. A plan that cannot be carried out fails at
    /// the next slot, which is hours after the click that caused it.
    #[tokio::test]
    async fn a_credential_in_use_is_not_forgotten_by_accident() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", "a-secret"),
                ("region", "us-east-1"),
                ("endpoint", ""),
            ])
            .send()
            .await;
        // The destination is written directly: saving it through the form
        // would ask the bucket, and there is no bucket in a test.
        crate::node::backups::store(
            &console.database,
            &crate::node::backups::Plan {
                destination: Some("s3://somebodys-bucket/nodes".into()),
                ..crate::node::backups::Plan::default()
            },
        )
        .await
        .expect("store the plan");

        let refused = console
            .harness
            .post("/node/backups/credentials/forget")
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(
            refused
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "{:?}",
            refused.header("location")
        );
        assert!(console.config.s3().is_some(), "still stored");

        // Pointed somewhere else, it goes.
        crate::node::backups::store(&console.database, &crate::node::backups::Plan::default())
            .await
            .expect("store the plan");
        console
            .harness
            .post("/node/backups/credentials/forget")
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(console.config.s3().is_none(), "and it is gone");
    }

    /// A failure is a value somebody can act on.
    ///
    /// A destination that has been refusing for a week is invisible in a
    /// journal nobody reads, so the reason is on the row — and from there
    /// on the page and in the attention card.
    #[tokio::test]
    async fn the_last_failure_is_on_the_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        crate::node::backups::record(
            &console.database,
            crate::clock::now_ms(),
            &Err("ssh: connect to host backups.example port 22: no route to host".into()),
        )
        .await
        .expect("record");

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("no route to host"), "{body}");
    }

    /// Settings adjusts; it does not report.
    ///
    /// The meters were on this page when it was `/nodes`, and Jorge asked
    /// twice for them to go: nothing on them is adjustable, and the same
    /// readings are on the node's own page in the network. The second ask
    /// came with a screenshot, and this is the assertion that was missing
    /// — I had checked the source and told him it was already done, which
    /// is not the same as knowing.
    #[tokio::test]
    async fn the_settings_page_has_no_meters_on_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        // **Every tab, not `/node`.** That path is a redirect now, and a
        // redirect's body contains no meter — so asking it would pass
        // whatever the tabs held, which is the shape that has bitten this
        // session twice already.
        let mut bodies = Vec::new();
        for tab in NodeTab::all() {
            bodies.push(
                console
                    .harness
                    .get(&tab.path())
                    .header("cookie", &cookie)
                    .send()
                    .await
                    .body,
            );
        }
        for body in &bodies {
            // The meter's own markup, not the word: `Memory` is a label a
            // future card could reuse, while `data-meter` and `data-bar`
            // are what a bar *is*.
            assert!(
                !body.contains("data-meter"),
                "a meter is on the settings page: {body}"
            );
            assert!(!body.contains(r#"data-bar="#), "and so is a bar: {body}");
        }
        // What it is for is still here, each on its own tab.
        assert!(
            bodies
                .iter()
                .any(|body| body.contains("/node/capabilities")),
            "the capability switch is on none of them"
        );
        assert!(
            bodies.iter().any(|body| body.contains("/node/certificate")),
            "the certificate is on none of them"
        );

        // And the readings are on the node's page, which is where they
        // belong — so this is a move and not a deletion.
        let node = console
            .harness
            .get(&console.node_path)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            node.contains("data-meter"),
            "the readings went nowhere: {node}"
        );
    }

    /// The point of the whole form: a node installed before its DNS
    /// was ready serves its own certificate, and there has to be a way
    /// back from that without editing a file and restarting.
    ///
    /// The button says "Save domain" now. It used to say "Request a
    /// certificate", which described one thing it sometimes did rather
    /// than the thing it always does — and on a node with no domain,
    /// which is this one, no request was possible at all.
    #[tokio::test]
    async fn the_node_page_offers_to_save_a_domain() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/node/domain")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Save domain"), "{body}");
        assert!(
            !body.contains("Request a certificate"),
            "and no longer promises a request: {body}"
        );
        assert!(
            body.contains("/node/certificate"),
            "the form posts somewhere"
        );
    }

    /// Every `data-when` has to name a control that exists in the page
    /// it is on. A typo in the condition is the failure mode of a
    /// declarative island: nothing throws, the field is simply hidden
    /// forever or shown forever, and the page looks fine.
    pub(crate) fn conditions_name_real_controls(body: &str) {
        let mut checked = 0;
        for attribute in ["data-when=\"", "data-required-when=\""] {
            for (index, _) in body.match_indices(attribute) {
                let rest = &body[index + attribute.len()..];
                let condition = &rest[..rest.find('"').expect("a closed attribute")];
                let name = condition.split('=').next().expect("a name");
                assert!(
                    body.contains(&format!("name=\"{name}\"")),
                    "{condition} names no control on this page"
                );
                checked += 1;
            }
        }
        assert!(checked > 0, "no conditional fields on this page");
    }

    /// The island hides; it does not build. Both paths are in the HTML
    /// the node produced, so the form works with scripting off — that
    /// is the whole of the rule this console kept when it stopped
    /// saying "no JavaScript".
    #[tokio::test]
    async fn the_certificate_paths_are_rendered_and_merely_hidden() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/node/domain")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("name=\"cert_path\""),
            "server-rendered: {body}"
        );
        assert!(body.contains("name=\"key_path\""), "{body}");
        assert!(
            body.contains("data-when=\"renew_with=file\""),
            "and conditioned: {body}"
        );
        conditions_name_real_controls(&body);
    }

    /// A node with no domain has nothing a public authority could
    /// certify, and that is a *state*, not a failure. It used to be
    /// indistinguishable from a node whose request had been refused,
    /// because only failures were recorded.
    #[tokio::test]
    async fn a_node_with_no_domain_says_so_rather_than_nothing() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let body = console
            .harness
            .get("/node/domain")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Self-signed"), "the badge: {body}");
        assert!(
            body.contains("no name an authority could"),
            "and what it means: {body}"
        );
    }

    /// Each state is one badge word, so the stream can replace it by
    /// assigning text. A state that rendered differently from the way
    /// it streams would be right until the first update.
    #[test]
    fn every_state_has_a_word_and_a_colour() {
        let states = [
            CertificateState::Local,
            CertificateState::Nameless,
            CertificateState::Working,
            CertificateState::Waiting,
            CertificateState::Trusted,
            CertificateState::Staging,
            CertificateState::Failed("refused".into()),
        ];
        let mut words = std::collections::BTreeSet::new();
        for state in &states {
            let (word, badge, dot) = state.badge();
            assert!(!word.is_empty() && !state.note().is_empty(), "{state:?}");
            assert!(
                badge.starts_with("badge") && dot.starts_with("dot"),
                "{state:?}"
            );
            words.insert(word);
        }
        assert_eq!(words.len(), states.len(), "two states read the same");
    }

    /// The reason a request is in flight has to reach the page while it
    /// is in flight — that is the whole point of the phase. Reading it
    /// from the last *error* could only ever describe attempts that had
    /// already finished badly.
    #[test]
    fn an_attempt_in_flight_outranks_what_was_stored() {
        let facts = super::super::CertificateFacts {
            issuer: "self-signed".into(),
            days_left: 0,
            trusted: false,
            source: crate::edge::certs::Source::SelfSigned,
        };
        let working = CertificateState::read(
            &facts,
            Some("node.example.com"),
            Some("refused last time".into()),
            crate::edge::acme::Phase::Working,
            false,
        );
        assert_eq!(working, CertificateState::Working);

        let settled = CertificateState::read(
            &facts,
            Some("node.example.com"),
            Some("refused last time".into()),
            crate::edge::acme::Phase::Idle,
            false,
        );
        assert_eq!(
            settled,
            CertificateState::Failed("refused last time".into())
        );

        // A node whose ACME is switched off is not "waiting" for
        // anything, and saying so would be an invitation to keep
        // looking at a page that will never change.
        let off = CertificateState::read(
            &facts,
            Some("node.example.com"),
            None,
            crate::edge::acme::Phase::Idle,
            true,
        );
        assert_eq!(off, CertificateState::Local);
    }

    /// A name that does not point here cannot be accepted: asking an
    /// authority to validate it spends one of five hourly attempts to
    /// be told what a lookup says for free.
    ///
    /// **And the refusal has to land where the form is.** This asserted
    /// `console.node_path` — the node's page in the network list, which
    /// has no error note and reads no `checked` parameter — so it
    /// passed for as long as the answer was being dropped on the floor.
    /// A test that restates what the code does catches nothing, and
    /// this one restated a redirect that had been wrong since before
    /// the node's settings grew tabs.
    ///
    /// Reported by Jorge, trying to set `node-0.tobaw.shop`: the
    /// refusal was correct and reached nobody.
    #[tokio::test]
    async fn a_name_that_does_not_resolve_here_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let response = console
            .harness
            .post("/node/certificate")
            .header("cookie", &cookie)
            // `.invalid` is reserved by RFC 2606 precisely so it can
            // never resolve — no network needed to know the answer.
            .form(&[("domain", "nowhere.invalid")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default();
        assert!(
            location.starts_with(&format!("{}?error=", NodeTab::Domain.path())),
            "the answer landed on {location}, away from the form that asked"
        );

        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default()).await,
            None,
            "a refused name must not be stored"
        );
    }

    /// Clearing it is how somebody goes back to a node with no public
    /// name — and it must not be gated on a lookup.
    #[tokio::test]
    async fn the_domain_can_be_cleared() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        crate::node::settings::set_domain(&console.database, Some("was.example"))
            .await
            .expect("set");

        console
            .harness
            .post("/node/certificate")
            .header("cookie", &cookie)
            .form(&[("domain", "")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default()).await,
            None
        );
    }

    /// Changing what the whole node answers to is not something a
    /// member of one project gets to do.
    #[tokio::test]
    async fn only_an_admin_may_change_it() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let member = console.joined_as(&admin, "member").await;

        crate::node::settings::set_domain(&console.database, Some("was.example"))
            .await
            .expect("set");

        let response = console
            .harness
            .post("/node/certificate")
            .header("cookie", &member)
            .form(&[("domain", "")])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
        assert_eq!(
            crate::node::settings::domain(&console.database, &Config::default())
                .await
                .as_deref(),
            Some("was.example"),
            "a member changed the node's domain"
        );
    }
}
