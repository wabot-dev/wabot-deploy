use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::domain::authz::node_user;
use crate::domain::values::destinations::Destinations;
use crate::features::projects::project_rest_controller::locate_for;
use crate::features::projects::project_settings_tabs_component::ProjectSettingsTab;
use crate::features::projects::set_egress_request::SetEgressRequest;
use crate::features::projects::set_exits_request::SetExitsRequest;
use crate::shell::auth::{
    back_with_error, field, read_form, see_other, signed_in, ticked, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct EgressRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl EgressRestController {
    #[post("/projects/:project/egress")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn save(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = locate_for(&self.state, &self.auth, &path).await? else {
            return Ok(see_other("/projects?error=no+such+project"));
        };
        let here = ProjectSettingsTab::Internet.path(&project.slug);
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = match Destinations::parse(field(&form, "destinations")) {
            Ok(destinations) => SetEgressRequest { destinations },
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        let saved = match node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_set_egress::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &allowed,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_set_egress::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &allowed,
                    &asked,
                )
                .await
            }
        };
        match saved {
            Ok(()) => Ok(see_other(&format!("{here}#destinations"))),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/exits")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn exits(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = locate_for(&self.state, &self.auth, &path).await? else {
            return Ok(see_other("/projects?error=no+such+project"));
        };
        let here = ProjectSettingsTab::Internet.path(&project.slug);
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = SetExitsRequest {
            exits: ticked(&form, "exit-"),
        };
        let saved = match node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_set_exits::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &allowed,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_set_exits::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &allowed,
                    &asked,
                )
                .await
            }
        };
        match saved {
            Ok(()) => Ok(see_other(&format!("{here}#exits"))),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::values::capability::Capability;
    use crate::domain::values::roles::ProjectRole;
    use crate::infra::repositories::{access, egress, projects};
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    async fn made(console: &Console, admin: &str) -> crate::domain::entities::project::Project {
        console
            .harness
            .post("/projects")
            .header("cookie", admin)
            .form(&[("name", "shop")])
            .send()
            .await;
        projects::find(&console.database, "shop")
            .await
            .expect("query")
            .expect("present")
    }

    #[tokio::test]
    async fn an_owner_closes_the_project_and_the_tab_says_what_is_left() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let project = made(&console, &admin).await;
        let tab = || async {
            console
                .harness
                .get("/projects/shop/settings/internet")
                .header("cookie", &admin)
                .send()
                .await
                .body
        };
        assert!(tab().await.contains("It may connect anywhere."));

        console
            .harness
            .post("/projects/shop/egress")
            .header("cookie", &admin)
            .form(&[("destinations", "203.0.113.5:443 payments")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        assert_eq!(
            egress::of_project(&console.database, &project.id)
                .await
                .expect("read")
                .text(),
            "203.0.113.5:443 payments"
        );
        let page = tab().await;
        assert!(page.contains("Only the destinations below."), "{page}");
        assert!(page.contains("203.0.113.5:443 payments"), "{page}");

        console
            .harness
            .post("/projects/shop/egress")
            .header("cookie", &admin)
            .form(&[("destinations", "")])
            .send()
            .await;
        assert!(tab().await.contains("Nothing leaves: the list is empty."));
    }

    #[tokio::test]
    async fn a_deployer_reads_the_list_and_does_not_decide_it() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let project = made(&console, &admin).await;
        let member = console.joined_as(&admin, "deployer").await;
        let person = crate::infra::accounts::all(&console.database)
            .await
            .expect("query")
            .into_iter()
            .find(|account| account.username == "deployer")
            .expect("joined");
        access::grant(
            &console.database,
            &person.id,
            &project.id,
            ProjectRole::Deployer,
        )
        .await
        .expect("granted");

        let page = console
            .harness
            .get("/projects/shop/settings/internet")
            .header("cookie", &member)
            .send()
            .await
            .body;
        assert!(page.contains("0.0.0.0/0"), "{page}");
        assert!(!page.contains("/projects/shop/egress"), "{page}");

        console
            .harness
            .post("/projects/shop/egress")
            .header("cookie", &member)
            .form(&[("destinations", "")])
            .send()
            .await;
        assert!(egress::of_project(&console.database, &project.id)
            .await
            .expect("read")
            .is_anywhere());
    }

    #[tokio::test]
    async fn a_line_that_cannot_be_enforced_returns_with_the_reason_and_changes_nothing() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let project = made(&console, &admin).await;

        let response = console
            .harness
            .post("/projects/shop/egress")
            .header("cookie", &admin)
            .form(&[("destinations", "203.0.113.5\n2001:db8::/32")])
            .send()
            .await;
        let location = response.header("location").expect("redirected").to_string();
        assert!(
            location.starts_with("/projects/shop/settings/internet?error="),
            "{location}"
        );
        assert!(location.contains("IPv6"), "{location}");
        assert!(egress::of_project(&console.database, &project.id)
            .await
            .expect("read")
            .is_anywhere());
    }

    async fn neighbour(console: &Console, id: &str, overlay: &str, allows: Vec<Capability>) {
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: Some("0.44.0".into()),
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: id.into(),
                name: format!("{id}.example"),
                kind: crate::infra::network::Kind::Private,
                overlay_ip: Some(overlay.into()),
                public_key: None,
                endpoint: None,
                allows,
                ca_pem: None,
                last_seen_at: None,
                is_self: false,
            },
        )
        .await
        .expect("node");
    }

    #[tokio::test]
    async fn an_owner_chooses_the_nodes_it_leaves_through_from_those_that_let_it() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let project = made(&console, &admin).await;
        let tab = || async {
            console
                .harness
                .get("/projects/shop/settings/internet")
                .header("cookie", &admin)
                .send()
                .await
                .body
        };
        assert!(tab()
            .await
            .contains("Each copy leaves from the node it runs on."));

        crate::infra::network::ensure_hub(
            &console.database,
            &crate::infra::config::Config::default(),
        )
        .await
        .expect("hub");
        neighbour(
            &console,
            "nd-exit",
            "10.42.0.7",
            vec![Capability::Host, Capability::Exit],
        )
        .await;
        neighbour(&console, "nd-host", "10.42.0.8", vec![Capability::Host]).await;
        let page = tab().await;
        assert!(page.contains("name=\"exit-nd-exit\""), "{page}");
        assert!(!page.contains("name=\"exit-nd-host\""), "{page}");

        let choose = |field: &'static str| {
            console
                .harness
                .post("/projects/shop/exits")
                .header("cookie", &admin)
                .form(&[(field, "1")])
                .send()
        };
        choose("exit-nd-exit")
            .await
            .assert_status(StatusCode::SEE_OTHER);
        assert_eq!(
            egress::exits_of(&console.database, &project.id)
                .await
                .expect("read"),
            vec!["10.42.0.7"
                .parse::<std::net::Ipv4Addr>()
                .expect("an address")]
        );
        let page = tab().await;
        assert!(
            page.contains("name=\"exit-nd-exit\" value=\"1\" checked"),
            "{page}"
        );

        choose("exit-nd-host").await;
        assert_eq!(
            egress::exits_of(&console.database, &project.id)
                .await
                .expect("read")
                .len(),
            1,
            "a node that does not let this one name it is refused, and nothing changes"
        );

        choose("unrelated").await;
        assert!(egress::exits_of(&console.database, &project.id)
            .await
            .expect("read")
            .is_empty());
    }
}
