use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::domain::authz::node_user;
use crate::features::services::service_rest_controller::locate_for;
use crate::shell::auth::{
    back_with_error, read_form, see_other, segments, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct ServiceRedirectRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl ServiceRedirectRestController {
    #[post("/projects/:project/services/:service/redirects")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = locate_for(&self.state, &self.auth, &path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let tab = format!(
            "/projects/{}/services/{}/settings/network",
            project.slug, service.slug
        );
        let form_page = format!(
            "/projects/{}/services/{}/redirects/new",
            project.slug, service.slug
        );
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked =
            crate::features::services::add_service_redirect_request::AddServiceRedirectRequest {
                hostname: form.get("hostname").cloned().unwrap_or_default(),
                to: form.get("to").cloned().unwrap_or_default(),
            };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&form_page, &reason.to_string()));
        }
        let added = match node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_add_service_redirect::run(
                    &self.state.database,
                    &account,
                    &service,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_add_service_redirect::run(
                    &self.state.database,
                    &account,
                    &service,
                    &asked,
                )
                .await
            }
        };
        if let Err(reason) = added {
            return Ok(back_with_error(&form_page, &reason));
        }
        self.state.deployer.sync_routes().await;
        self.state.deployer.tell_edges(&service).await;
        Ok(see_other(&format!("{tab}#redirects")))
    }

    #[post("/projects/:project/services/:service/redirects/:hostname/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = locate_for(&self.state, &self.auth, &path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let tab = format!(
            "/projects/{}/services/{}/settings/network",
            project.slug, service.slug
        );
        let parts = segments(&path);
        let ["projects", _, "services", _, "redirects", hostname, "delete"] = parts.as_slice()
        else {
            return Ok(see_other(&tab));
        };
        let hostname = hostname.to_string();
        let removed = match node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_delete_service_redirect::run(
                    &self.state.database,
                    &account,
                    &service,
                    &hostname,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_delete_service_redirect::run(
                    &self.state.database,
                    &account,
                    &service,
                    &hostname,
                )
                .await
            }
        };
        if let Err(reason) = removed {
            return Ok(back_with_error(&tab, &reason));
        }
        self.state.deployer.sync_routes().await;
        Ok(see_other(&format!("{tab}#redirects")))
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::network::errand;
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    const SERVICE: &str = "/projects/my-api/services/web";
    const TAB: &str = "/projects/my-api/services/web/settings/network";

    /// A service whose port answers at `localhost`, and a node that names
    /// can be checked against: `localhost` and `127.0.0.1` both reach it.
    async fn named_service(console: &Console, cookie: &str) {
        console
            .harness
            .post("/projects")
            .header("cookie", cookie)
            .form(&[("name", "My API")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        console
            .harness
            .post("/projects/my-api/services")
            .header("cookie", cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-theedge".into(),
                name: "edge.example".into(),
                kind: crate::infra::network::Kind::Public,
                endpoint: Some("localhost:443".into()),
                public_key: Some("0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
                allows: vec![crate::domain::values::capability::Capability::Edge],
                ca_pem: Some(
                    "-----BEGIN CERTIFICATE-----\nnot real\n-----END CERTIFICATE-----\n".into(),
                ),
            },
        )
        .await
        .expect("a node names can reach");
    }

    async fn give_it_a_name(console: &Console, cookie: &str) {
        console
            .harness
            .post(&format!("{SERVICE}/ports"))
            .header("cookie", cookie)
            .form(&[
                ("container_port", "8080"),
                ("https", "1"),
                ("hostname", "localhost"),
                ("renew_with", "self_signed"),
            ])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
    }

    async fn add(console: &Console, cookie: &str, hostname: &str, to: &str) -> String {
        console
            .harness
            .post(&format!("{SERVICE}/redirects"))
            .header("cookie", cookie)
            .form(&[("hostname", hostname), ("to", to)])
            .send()
            .await
            .header("location")
            .unwrap_or_default()
            .to_string()
    }

    fn told(console_errands: &[errand::Record], hostname: &str) -> Option<errand::Edge> {
        console_errands
            .iter()
            .filter(|order| order.node_id == "nd-theedge" && order.kind == errand::Kind::Edge)
            .filter_map(|order| serde_json::from_value::<errand::Edge>(order.payload.clone()).ok())
            .find(|edge| edge.hostname == hostname)
    }

    /// The redirect is made on a page of its own and listed on the Network
    /// tab, and the node its name reaches is told to answer for it with a
    /// redirect rather than with the service.
    #[tokio::test]
    async fn a_redirect_is_made_on_its_own_page_and_told_to_the_node_its_name_reaches() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        named_service(&console, &cookie).await;

        let tab = console
            .harness
            .get(TAB)
            .header("cookie", &cookie)
            .send()
            .await;
        tab.assert_ok();
        assert!(
            !tab.body
                .contains(&format!("href=\"{SERVICE}/redirects/new\"")),
            "offered with nothing to redirect to"
        );
        let early = console
            .harness
            .get(&format!("{SERVICE}/redirects/new"))
            .header("cookie", &cookie)
            .send()
            .await;
        assert_eq!(early.header("location"), Some(TAB));

        give_it_a_name(&console, &cookie).await;
        let tab = console
            .harness
            .get(TAB)
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(tab
            .body
            .contains(&format!("href=\"{SERVICE}/redirects/new\"")));
        let form = console
            .harness
            .get(&format!("{SERVICE}/redirects/new"))
            .header("cookie", &cookie)
            .send()
            .await;
        form.assert_ok();
        assert!(form
            .body
            .contains(&format!("action=\"{SERVICE}/redirects\"")));
        assert!(form.body.contains("<option value=\"localhost\">"));

        let location = add(&console, &cookie, "127.0.0.1", "localhost").await;
        assert_eq!(location, format!("{TAB}#redirects"));
        let made = crate::infra::repositories::redirects::all(&console.database)
            .await
            .expect("redirects");
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].hostname, "127.0.0.1");
        assert_eq!(made[0].target, "localhost");

        let tab = console
            .harness
            .get(TAB)
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(tab
            .body
            .contains(&format!("action=\"{SERVICE}/redirects/127.0.0.1/delete\"")));

        let errands = errand::all(&console.database).await.expect("errands");
        let edge = told(&errands, "127.0.0.1").expect("the node the name reaches was told");
        assert_eq!(edge.redirect_to.as_deref(), Some("localhost"));
    }

    #[tokio::test]
    async fn a_refused_redirect_says_why_on_its_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        named_service(&console, &cookie).await;
        give_it_a_name(&console, &cookie).await;
        let form = format!("{SERVICE}/redirects/new?error=");

        for (hostname, to, why) in [
            (
                "127.0.0.1",
                "elsewhere.example",
                "a+redirect+goes+to+one+of+this",
            ),
            ("localhost", "localhost", "a+name+cannot+redirect+to+itself"),
            ("nowhere.invalid", "localhost", ""),
        ] {
            let location = add(&console, &cookie, hostname, to).await;
            assert!(
                location.starts_with(&form) && location.contains(why),
                "{hostname} → {to}: {location}"
            );
        }
        assert!(
            crate::infra::repositories::redirects::all(&console.database)
                .await
                .expect("redirects")
                .is_empty()
        );
    }

    /// The rows that told the other node about the name are the ones a
    /// delete removes, so nothing later would tell it to stop — unless
    /// something asks what each node was last told, which the route sync
    /// does.
    #[tokio::test]
    async fn removing_a_redirect_tells_the_node_that_answered_for_it_to_stop() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        named_service(&console, &cookie).await;
        give_it_a_name(&console, &cookie).await;
        add(&console, &cookie, "127.0.0.1", "localhost").await;
        // What a deployment sends once a copy runs; nothing runs here.
        let serving = serde_json::to_value(errand::Edge {
            hostname: "127.0.0.1".into(),
            upstreams: vec!["10.42.0.3:30000".into()],
            peers: Vec::new(),
            secret: None,
            redirect_to: Some("localhost".into()),
        })
        .expect("payload");
        errand::queue(
            &console.database,
            "nd-theedge",
            errand::Kind::Edge,
            &serving,
        )
        .await
        .expect("served");

        let removed = console
            .harness
            .post(&format!("{SERVICE}/redirects/127.0.0.1/delete"))
            .header("cookie", &cookie)
            .send()
            .await;
        assert_eq!(
            removed.header("location"),
            Some(format!("{TAB}#redirects").as_str())
        );
        assert!(
            crate::infra::repositories::redirects::all(&console.database)
                .await
                .expect("redirects")
                .is_empty()
        );
        assert!(
            !errand::names_told(&console.database)
                .await
                .expect("told")
                .contains(&("nd-theedge".to_string(), "127.0.0.1".to_string())),
            "the node still answers for the redirect's name"
        );
    }
}
