use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::domain::authz::node_user;
use crate::shell::auth::{
    back_with_error, read_form, see_other, segments, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct ZoneRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl ZoneRestController {
    #[post("/projects/:project/zones")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create_zone(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(project) = segments(&path).get(1).map(|project| project.to_string()) else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{project}/zones");
        let form_page = format!("{here}/new");
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = crate::features::dns::create_zone_request::CreateZoneRequest {
            project: project.clone(),
            name: form.get("name").cloned().unwrap_or_default(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&form_page, &reason.to_string()));
        }
        let created = match node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_create_zone::run(
                    &self.state.database,
                    &self.state.container,
                    &account,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_create_zone::run(
                    &self.state.database,
                    &self.state.container,
                    &account,
                    &asked,
                )
                .await
            }
        };
        match created {
            Ok(created) => Ok(see_other(&format!("{here}/{}", created.name))),
            Err(reason) => Ok(back_with_error(&form_page, &reason)),
        }
    }

    #[post("/projects/:project/zones/:zone/records")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_record(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let parts = segments(&path);
        let (Some(project), Some(zone)) = (
            parts.get(1).map(|part| part.to_string()),
            parts.get(3).map(|part| part.to_string()),
        ) else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{project}/zones/{zone}");
        let form_page = format!("{here}/records/new");
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let field = |name: &str| form.get(name).cloned().unwrap_or_default();
        let asked = crate::features::dns::add_record_request::AddRecordRequest {
            project,
            zone,
            name: field("name"),
            kind: field("kind"),
            value: field("value"),
            priority: field("priority"),
            ttl: field("ttl"),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&form_page, &reason.to_string()));
        }
        let added = match node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_add_record::run(
                    &self.state.database,
                    &self.state.container,
                    &account,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_add_record::run(
                    &self.state.database,
                    &self.state.container,
                    &account,
                    &asked,
                )
                .await
            }
        };
        match added {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&form_page, &reason)),
        }
    }

    #[post("/projects/:project/zones/:zone/records/:record/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove_record(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let parts = segments(&path);
        let (Some(project), Some(zone), Some(record)) = (parts.get(1), parts.get(3), parts.get(5))
        else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{project}/zones/{zone}");
        let removed = match node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_remove_record::run(
                    &self.state.database,
                    &self.state.container,
                    &account,
                    project,
                    zone,
                    record,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_remove_record::run(
                    &self.state.database,
                    &self.state.container,
                    &account,
                    project,
                    zone,
                    record,
                )
                .await
            }
        };
        match removed {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/zones/:zone/check")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn check_delegation(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let parts = segments(&path);
        let (Some(project), Some(zone)) = (parts.get(1), parts.get(3)) else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{project}/zones/{zone}");
        let checked = match node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_check_delegation::run(
                    &self.state.database,
                    &account,
                    project,
                    zone,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_check_delegation::run(
                    &self.state.database,
                    &account,
                    project,
                    zone,
                )
                .await
            }
        };
        match checked {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/zones/:zone/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete_zone(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let parts = segments(&path);
        let (Some(project), Some(zone)) = (parts.get(1), parts.get(3)) else {
            return Ok(see_other("/"));
        };
        let deleted = match node_user::belongs_here(&account) {
            true => {
                crate::features::dns::node_user_delete_zone::run(
                    &self.state.database,
                    &self.state.container,
                    &account,
                    project,
                    zone,
                )
                .await
            }
            false => {
                crate::features::dns::tenant_user_delete_zone::run(
                    &self.state.database,
                    &self.state.container,
                    &account,
                    project,
                    zone,
                )
                .await
            }
        };
        match deleted {
            Ok(()) => Ok(see_other(&format!("/projects/{project}/zones"))),
            Err(reason) => Ok(back_with_error(
                &format!("/projects/{project}/zones/{zone}"),
                &reason,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::infra::repositories::zones;
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    async fn project(console: &Console, cookie: &str, name: &str) {
        console
            .harness
            .post("/projects")
            .header("cookie", cookie)
            .form(&[("name", name)])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
    }

    async fn add_zone(
        console: &Console,
        cookie: &str,
        project: &str,
        name: &str,
    ) -> wabot::testing::TestResponse {
        console
            .harness
            .post(&format!("/projects/{project}/zones"))
            .header("cookie", cookie)
            .form(&[("name", name)])
            .send()
            .await
    }

    async fn add_record(
        console: &Console,
        cookie: &str,
        zone: &str,
        fields: &[(&str, &str)],
    ) -> wabot::testing::TestResponse {
        console
            .harness
            .post(&format!("/projects/site/zones/{zone}/records"))
            .header("cookie", cookie)
            .form(fields)
            .send()
            .await
    }

    /// The page and the name server read the same rows: what the form on a
    /// project's page saved is what a resolver is told.
    #[tokio::test]
    async fn a_zone_made_in_a_project_is_what_the_name_server_answers_from() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        project(&console, &admin, "Site").await;

        let created = add_zone(&console, &admin, "site", "TeresaAsprilla.com").await;
        assert_eq!(
            created.header("location"),
            Some("/projects/site/zones/teresaasprilla.com")
        );
        let added = add_record(
            &console,
            &admin,
            "teresaasprilla.com",
            &[
                ("name", "@"),
                ("kind", "A"),
                ("value", "209.50.48.176"),
                ("ttl", ""),
            ],
        )
        .await;
        assert_eq!(
            added.header("location"),
            Some("/projects/site/zones/teresaasprilla.com")
        );

        let list = console
            .harness
            .get("/projects/site/zones")
            .header("cookie", &admin)
            .send()
            .await;
        list.assert_ok();
        assert!(list
            .body
            .contains("href=\"/projects/site/zones/teresaasprilla.com\""));
        let page = console
            .harness
            .get("/projects/site/zones/teresaasprilla.com")
            .header("cookie", &admin)
            .send()
            .await;
        page.assert_ok();
        assert!(
            page.body.contains("209.50.48.176"),
            "the record is on its page"
        );

        let served = crate::infra::dns::what_this_node_serves(&console.database)
            .await
            .expect("served");
        assert_eq!(served.len(), 1);
        assert_eq!(served[0].zone.name.as_str(), "teresaasprilla.com");
        assert_eq!(served[0].records.len(), 1);
    }

    #[tokio::test]
    async fn a_refused_record_says_why_and_saves_nothing() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        project(&console, &admin, "Site").await;
        add_zone(&console, &admin, "site", "teresaasprilla.com").await;

        let refused = add_record(
            &console,
            &admin,
            "teresaasprilla.com",
            &[("name", "@"), ("kind", "A"), ("value", "not-an-address")],
        )
        .await;
        let location = refused.header("location").unwrap_or_default();
        assert!(
            location.starts_with(
                "/projects/site/zones/teresaasprilla.com/records/new?error=an+A+record"
            ),
            "the refusal goes back to the form it came from: {location}"
        );
        let zone = zones::find_by_name(&console.database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("zone");
        assert!(zones::records(&console.database, &zone)
            .await
            .expect("records")
            .is_empty());
    }

    /// A zone is added on a page of its own too, with the name servers to
    /// set beside the form — what somebody needs before they add a domain,
    /// not after.
    #[tokio::test]
    async fn a_zone_is_added_on_a_page_of_its_own() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        project(&console, &admin, "Site").await;

        let list = console
            .harness
            .get("/projects/site/zones")
            .header("cookie", &admin)
            .send()
            .await;
        list.assert_ok();
        assert!(list.body.contains("href=\"/projects/site/zones/new\""));
        assert!(
            !list.body.contains("name=\"name\""),
            "the list still carries the zone form"
        );

        let form = console
            .harness
            .get("/projects/site/zones/new")
            .header("cookie", &admin)
            .send()
            .await;
        form.assert_ok();
        assert!(form.body.contains("action=\"/projects/site/zones\""));
        assert!(form.body.contains("name=\"name\""));
        assert!(
            form.body.contains("At your domain"),
            "the name servers to set are not beside the form"
        );

        let refused = add_zone(&console, &admin, "site", "localhost").await;
        let location = refused.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/projects/site/zones/new?error="),
            "the refusal goes back to the form it came from: {location}"
        );

        let viewer = console.joined_as(&admin, "viewer").await;
        let account = crate::infra::accounts::all(&console.database)
            .await
            .expect("accounts")
            .into_iter()
            .find(|account| account.username == "viewer")
            .expect("the viewer");
        let site = crate::infra::repositories::projects::find(&console.database, "site")
            .await
            .expect("read")
            .expect("site");
        crate::infra::repositories::access::grant(
            &console.database,
            &account.id,
            &site.id,
            crate::domain::values::roles::ProjectRole::Viewer,
        )
        .await
        .expect("a viewer of site");
        let bounced = console
            .harness
            .get("/projects/site/zones/new")
            .header("cookie", &viewer)
            .send()
            .await;
        assert_eq!(
            bounced.header("location"),
            Some("/projects/site/zones"),
            "a viewer was shown the form"
        );
        let list = console
            .harness
            .get("/projects/site/zones")
            .header("cookie", &viewer)
            .send()
            .await;
        assert!(!list.body.contains("href=\"/projects/site/zones/new\""));
    }

    /// A record is added on a page of its own, the way a service or a
    /// database is made: the zone's page lists them and offers the button,
    /// and the form is somewhere else. It was under the list, which Jorge
    /// called horrible.
    #[tokio::test]
    async fn a_record_is_added_on_a_page_of_its_own() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        project(&console, &admin, "Site").await;
        add_zone(&console, &admin, "site", "teresaasprilla.com").await;

        let zone = console
            .harness
            .get("/projects/site/zones/teresaasprilla.com")
            .header("cookie", &admin)
            .send()
            .await;
        zone.assert_ok();
        assert!(zone
            .body
            .contains("href=\"/projects/site/zones/teresaasprilla.com/records/new\""));
        assert!(
            !zone.body.contains("name=\"value\""),
            "the zone's page still carries the record form"
        );

        let form = console
            .harness
            .get("/projects/site/zones/teresaasprilla.com/records/new")
            .header("cookie", &admin)
            .send()
            .await;
        form.assert_ok();
        assert!(form
            .body
            .contains("action=\"/projects/site/zones/teresaasprilla.com/records\""));
        assert!(form.body.contains("name=\"value\""));
        assert!(form
            .body
            .contains("href=\"/projects/site/zones/teresaasprilla.com\""));

        let viewer = console.joined_as(&admin, "viewer").await;
        let account = crate::infra::accounts::all(&console.database)
            .await
            .expect("accounts")
            .into_iter()
            .find(|account| account.username == "viewer")
            .expect("the viewer");
        let site = crate::infra::repositories::projects::find(&console.database, "site")
            .await
            .expect("read")
            .expect("site");
        crate::infra::repositories::access::grant(
            &console.database,
            &account.id,
            &site.id,
            crate::domain::values::roles::ProjectRole::Viewer,
        )
        .await
        .expect("a viewer of site");
        let refused = console
            .harness
            .get("/projects/site/zones/teresaasprilla.com/records/new")
            .header("cookie", &viewer)
            .send()
            .await;
        assert_eq!(
            refused.header("location"),
            Some("/projects/site/zones/teresaasprilla.com"),
            "a viewer was shown the form"
        );
        let page = console
            .harness
            .get("/projects/site/zones/teresaasprilla.com")
            .header("cookie", &viewer)
            .send()
            .await;
        page.assert_ok();
        assert!(
            !page
                .body
                .contains("href=\"/projects/site/zones/teresaasprilla.com/records/new\""),
            "a viewer was offered the button"
        );
    }

    #[tokio::test]
    async fn a_deleted_zone_is_answered_by_nobody() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        project(&console, &admin, "Site").await;
        add_zone(&console, &admin, "site", "teresaasprilla.com").await;
        add_record(
            &console,
            &admin,
            "teresaasprilla.com",
            &[
                ("name", "www"),
                ("kind", "CNAME"),
                ("value", "teresaasprilla.com"),
            ],
        )
        .await;
        let zone = zones::find_by_name(&console.database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("zone");
        let record = zones::records(&console.database, &zone)
            .await
            .expect("records")[0]
            .id
            .clone();

        console
            .harness
            .post(&format!(
                "/projects/site/zones/teresaasprilla.com/records/{record}/delete"
            ))
            .header("cookie", &admin)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        assert!(zones::records(&console.database, &zone)
            .await
            .expect("records")
            .is_empty());

        let deleted = console
            .harness
            .post("/projects/site/zones/teresaasprilla.com/delete")
            .header("cookie", &admin)
            .send()
            .await;
        assert_eq!(deleted.header("location"), Some("/projects/site/zones"));
        assert!(crate::infra::dns::what_this_node_serves(&console.database)
            .await
            .expect("served")
            .is_empty());
    }

    /// A zone is reached through the project that holds it. Through
    /// another, it reads as no zone at all — a project somebody is in must
    /// not be a window onto the domains of one they are not.
    #[tokio::test]
    async fn a_zone_is_not_reached_through_another_project() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        project(&console, &admin, "Site").await;
        project(&console, &admin, "Other").await;
        add_zone(&console, &admin, "site", "teresaasprilla.com").await;

        let page = console
            .harness
            .get("/projects/other/zones/teresaasprilla.com")
            .header("cookie", &admin)
            .send()
            .await;
        assert_eq!(page.header("location"), Some("/projects/other/zones"));

        let refused = console
            .harness
            .post("/projects/other/zones/teresaasprilla.com/delete")
            .header("cookie", &admin)
            .send()
            .await;
        let location = refused.header("location").unwrap_or_default();
        assert!(
            location.contains("error=there+is+no+such+zone"),
            "{location}"
        );
        assert!(zones::find_by_name(&console.database, "teresaasprilla.com")
            .await
            .expect("read")
            .is_some());
    }

    /// The machine's operator chooses who answers, and only on the
    /// machine's console: the box for a node that never agreed is disabled,
    /// which is a courtesy, and this is the check that counts, on the POST.
    #[tokio::test]
    async fn only_a_node_that_agreed_can_be_chosen_to_answer() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        project(&console, &admin, "Site").await;
        add_zone(&console, &admin, "site", "teresaasprilla.com").await;
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-stranger".into(),
                name: "node-9.example.net".into(),
                kind: crate::infra::network::Kind::Public,
                endpoint: Some("node-9.example.net:443".into()),
                public_key: None,
                overlay_ip: Some("10.42.0.99".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("a node that never agreed");
        let me = crate::infra::network::all(&console.database)
            .await
            .expect("nodes")
            .into_iter()
            .find(|node| node.is_self)
            .expect("this node")
            .id;

        let page = console
            .harness
            .get("/projects/site/zones/teresaasprilla.com")
            .header("cookie", &admin)
            .send()
            .await;
        assert!(
            page.body
                .contains("action=\"/projects/site/zones/teresaasprilla.com/servers\""),
            "the operator is offered the choice"
        );

        let forged = console
            .harness
            .post("/projects/site/zones/teresaasprilla.com/servers")
            .header("cookie", &admin)
            .form(&[
                (format!("server-{me}").as_str(), "1"),
                ("server-nd-stranger", "1"),
            ])
            .send()
            .await;
        let location = forged.header("location").unwrap_or_default();
        assert!(
            location.contains("error=that+node+has+not+agreed"),
            "{location}"
        );

        let zone = zones::find_by_name(&console.database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("zone");
        assert_eq!(
            zones::servers(&console.database, &zone.id)
                .await
                .expect("servers"),
            vec![me],
            "a refused choice changed nothing"
        );
    }

    /// An organisation's console is a different router, and both halves of
    /// that are the claim: a project's zone doors are there, and the
    /// machine's — choosing name servers, the node's DNS tab, the old
    /// `/dns` — are not, by construction.
    #[tokio::test]
    async fn an_organisations_console_has_zones_and_no_choice_of_servers() {
        let console = Console::new().await;

        for path in ["/projects/acme/zones", "/projects/acme/zones/acme.example"] {
            let page = console.tenant.get(path).send().await;
            assert_ne!(page.status, StatusCode::NOT_FOUND, "{path} is not there");
        }
        let add = console
            .tenant
            .post("/projects/acme/zones")
            .form(&[("name", "acme.example")])
            .send()
            .await;
        assert_ne!(
            add.status,
            StatusCode::NOT_FOUND,
            "the organisation cannot add a zone"
        );

        let choose = console
            .tenant
            .post("/projects/acme/zones/acme.example/servers")
            .form(&[("server-nd-x", "1")])
            .send()
            .await;
        assert_eq!(
            choose.status,
            StatusCode::NOT_FOUND,
            "an organisation can choose name servers"
        );
        for path in ["/dns", "/node/dns"] {
            let page = console.tenant.get(path).send().await;
            assert_eq!(page.status, StatusCode::NOT_FOUND, "{path} answers");
        }
    }
}
