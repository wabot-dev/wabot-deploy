//! What this machine's settings page posts to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::console::auth::{signed_in, SessionMiddleware};
use crate::console::ConsoleState;
use crate::features::node::node_user_node_settings_page::NodeTab;

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
            return Ok(crate::console::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::console::auth::see_other("/"));
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

        let form = match crate::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        // "Try again" is the same form with nothing changed. It exists
        // because the usual fix for a refusal is somewhere else — a DNS
        // record, a firewall — and making somebody re-type the domain to
        // retry reads as though the domain were what was wrong.
        if form.contains_key("retry") {
            self.state.certificates.now();
            return Ok(crate::console::auth::see_other(here));
        }

        let settled = crate::features::node::node_user_set_node_domain::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
            crate::console::auth::field(&form, "domain"),
        )
        .await;

        // The loop owns issuance — both the local certificate carrying
        // the new name and the public one. Asking it here is what makes
        // saving a domain reissue rather than merely record.
        match settled {
            Ok(crate::features::node::domain_settled_response::DomainSettledResponse::Named(
                domain,
            )) => {
                self.state.certificates.now();
                Ok(crate::console::auth::see_other(&format!(
                    "{here}?{}",
                    form_urlencoded::Serializer::new(String::new())
                        .append_pair("checked", &format!("{domain} resolves here."))
                        .finish()
                )))
            }
            Ok(crate::features::node::domain_settled_response::DomainSettledResponse::Cleared) => {
                self.state.certificates.now();
                Ok(crate::console::auth::back_with_error(
                    here,
                    "the domain was cleared — this node serves its own certificate",
                ))
            }
            Err(reason) => Ok(crate::console::auth::back_with_error(here, &reason)),
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
            return Ok(crate::console::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::console::auth::see_other("/"));
        }
        let form = match crate::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        if let Err(reason) = crate::features::node::node_user_set_reserves::run(
            &self.state.database,
            &self.state.deployer,
            &account,
            crate::console::auth::field(&form, "memory"),
            crate::console::auth::field(&form, "cpu"),
        )
        .await
        {
            return Ok(crate::console::auth::back_with_error(here, &reason));
        }

        Ok(crate::console::auth::see_other(&format!("{here}#reserves")))
    }

    #[post("/node/capabilities")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_capabilities(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Allows.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::console::auth::see_other("/sign-in"));
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
            return Ok(crate::console::auth::see_other("/"));
        }
        let form = match crate::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked = crate::features::node::set_capabilities_request::SetCapabilitiesRequest {
            provides: crate::console::auth::capabilities_ticked(&form, ""),
            owns: form.contains_key("own"),
        };
        if let Err(reason) = crate::features::node::node_user_set_capabilities::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
            &asked,
        )
        .await
        {
            return Ok(crate::console::auth::back_with_error(here, &reason));
        }

        Ok(crate::console::auth::see_other(here))
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
            return Ok(crate::console::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::console::auth::see_other("/"));
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

        let form = match crate::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // The node's own name, or the fallback while it has none — the
        // same key the renewal loop will look this up under.
        let name = match crate::features::node::node_user_set_certificate_source::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            Ok(name) => name,
            Err(reason) => return Ok(crate::console::auth::back_with_error(here, &reason)),
        };

        let renew_with = match crate::console::auth::source_from(&form, &name) {
            Ok(renew_with) => renew_with,
            Err(reason) => return Ok(crate::console::auth::back_with_error(here, &reason)),
        };

        if let Err(error) = crate::commands::store_certificate_source::run(
            &self.state.database,
            &self.state.config,
            &name,
            &renew_with,
        )
        .await
        {
            return Ok(crate::console::auth::back_with_error(here, &error));
        }
        // The loop installs it. Same reason as saving a domain: this
        // console asks, it does not issue.
        self.state.certificates.now();

        Ok(crate::console::auth::see_other(here))
    }
}

#[cfg(test)]
mod tests {
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
        crate::console::tests::conditions_name_real_controls(&body);
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
        let facts = crate::console::CertificateFacts {
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
