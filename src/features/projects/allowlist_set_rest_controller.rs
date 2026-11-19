use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::domain::authz::node_user;
use crate::domain::values::allowlist::Allowlist;
use crate::domain::values::slug::Slug;
use crate::features::projects::project_rest_controller::locate_for;
use crate::features::projects::project_settings_tabs_component::ProjectSettingsTab;
use crate::shell::auth::{
    back_with_error, field, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct AllowlistSetRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl AllowlistSetRestController {
    #[post("/projects/:project/allowlists")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = locate_for(&self.state, &self.auth, &path).await? else {
            return Ok(see_other("/projects?error=no+such+project"));
        };
        let here = ProjectSettingsTab::Allowlists.path(&project.slug);
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = match Allowlist::parse(field(&form, "allowlist")) {
            Ok(list) => {
                crate::features::projects::create_allowlist_set_request::CreateAllowlistSetRequest {
                    name: Slug::of(field(&form, "name")),
                    list,
                }
            }
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        let made = match node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_create_allowlist_set::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_create_allowlist_set::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    &asked,
                )
                .await
            }
        };
        match made {
            Ok(()) => Ok(see_other(&format!("{here}#list-{}", asked.name))),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/allowlists/:list")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn save(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, segments)) = locate_for(&self.state, &self.auth, &path).await?
        else {
            return Ok(see_other("/projects?error=no+such+project"));
        };
        let here = ProjectSettingsTab::Allowlists.path(&project.slug);
        let name = segments.get(3).copied().unwrap_or_default().to_string();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = match Allowlist::parse(field(&form, "allowlist")) {
            Ok(list) => {
                crate::features::projects::set_allowlist_set_request::SetAllowlistSetRequest {
                    list,
                }
            }
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        let saved = match node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_set_allowlist_set::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &allowed,
                    &name,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_set_allowlist_set::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &allowed,
                    &name,
                    &asked,
                )
                .await
            }
        };
        match saved {
            Ok(()) => Ok(see_other(&format!("{here}#list-{name}"))),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/allowlists/:list/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, segments)) = locate_for(&self.state, &self.auth, &path).await?
        else {
            return Ok(see_other("/projects?error=no+such+project"));
        };
        let here = ProjectSettingsTab::Allowlists.path(&project.slug);
        let name = segments.get(3).copied().unwrap_or_default().to_string();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let deleted = match node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_delete_allowlist_set::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    &name,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_delete_allowlist_set::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    &name,
                )
                .await
            }
        };
        match deleted {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::domain::entities::project::Project;
    use crate::domain::entities::service::Service;
    use crate::domain::values::allowlist::Door;
    use crate::infra::repositories::{allowlist_sets, allowlists, projects, services};
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    async fn shop(console: &Console, cookie: &str, slug: &str) -> (Project, Service) {
        console
            .harness
            .post("/projects")
            .header("cookie", cookie)
            .form(&[("name", slug)])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        let project = projects::find(&console.database, slug)
            .await
            .expect("query")
            .expect("made");
        let api = services::create(
            &console.database,
            &project.id,
            "api",
            "hub.example.com/shop/api@sha256:abc",
            &[],
        )
        .await
        .expect("service");
        (project, api)
    }

    async fn post(console: &Console, cookie: &str, path: &str, form: &[(&str, &str)]) -> String {
        let response = console
            .harness
            .post(path)
            .header("cookie", cookie)
            .form(form)
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        response.header("location").unwrap_or_default().to_string()
    }

    async fn using(console: &Console, cookie: &str, sets: &[&str]) -> String {
        let names: Vec<String> = sets.iter().map(|id| format!("list-{id}")).collect();
        let form: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "1")).collect();
        post(
            console,
            cookie,
            "/projects/shop/services/api/allowlist-sets",
            &form,
        )
        .await
    }

    async fn admits(console: &Console, service: &Service, address: &str) -> bool {
        let loaded = allowlists::all(&console.database).await.expect("all");
        loaded
            .get(&Door::Service(service.id.clone()))
            .is_some_and(|networks| {
                crate::domain::values::allowlist::admits(
                    networks,
                    address.parse().expect("an address"),
                )
            })
    }

    #[tokio::test]
    async fn a_list_a_service_uses_lets_its_lines_in_and_follows_every_edit() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let (project, api) = shop(&console, &cookie, "shop").await;
        post(
            &console,
            &cookie,
            "/projects/shop/allowlists",
            &[("name", "office"), ("allowlist", "203.0.113.0/24 office")],
        )
        .await;
        let office = allowlist_sets::find(&console.database, &project.id, "office")
            .await
            .expect("read")
            .expect("made");
        assert!(!admits(&console, &api, "203.0.113.9").await);

        let back = using(&console, &cookie, &[&office.id]).await;
        assert!(!back.contains("error="), "{back}");
        assert!(admits(&console, &api, "203.0.113.9").await);

        let back = post(
            &console,
            &cookie,
            "/projects/shop/allowlists/office",
            &[("allowlist", "198.51.100.0/24 new office")],
        )
        .await;
        assert!(!back.contains("error="), "{back}");
        assert!(!admits(&console, &api, "203.0.113.9").await);
        assert!(admits(&console, &api, "198.51.100.1").await);

        let page = console
            .harness
            .get("/projects/shop/settings/allowlists")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(page.contains("198.51.100.0/24 new office"), "{page}");
        assert!(page.contains(">api<") || page.contains(" api<"), "{page}");
        let tab = console
            .harness
            .get("/projects/shop/services/api/settings/access")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            tab.contains(&format!(r#"name="list-{}" value="1" checked"#, office.id)),
            "{tab}"
        );
    }

    #[tokio::test]
    async fn a_service_uses_only_its_own_projects_lists() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let (_, api) = shop(&console, &cookie, "shop").await;
        let (other, _) = shop(&console, &cookie, "other").await;
        post(
            &console,
            &cookie,
            "/projects/other/allowlists",
            &[("name", "theirs"), ("allowlist", "0.0.0.0/0\n::/0")],
        )
        .await;
        let theirs = allowlist_sets::find(&console.database, &other.id, "theirs")
            .await
            .expect("read")
            .expect("made");

        let back = using(&console, &cookie, &[&theirs.id]).await;

        assert!(back.contains("error="), "{back}");
        assert!(!admits(&console, &api, "192.0.2.1").await);
    }

    #[tokio::test]
    async fn a_list_somebody_uses_is_not_deleted() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let (project, _) = shop(&console, &cookie, "shop").await;
        post(
            &console,
            &cookie,
            "/projects/shop/allowlists",
            &[("name", "office"), ("allowlist", "203.0.113.0/24")],
        )
        .await;
        let office = allowlist_sets::find(&console.database, &project.id, "office")
            .await
            .expect("read")
            .expect("made");
        using(&console, &cookie, &[&office.id]).await;

        let back = post(
            &console,
            &cookie,
            "/projects/shop/allowlists/office/delete",
            &[],
        )
        .await;
        assert!(back.contains("error="), "{back}");
        assert!(
            allowlist_sets::find(&console.database, &project.id, "office")
                .await
                .expect("read")
                .is_some()
        );

        using(&console, &cookie, &[]).await;
        post(
            &console,
            &cookie,
            "/projects/shop/allowlists/office/delete",
            &[],
        )
        .await;
        assert!(
            allowlist_sets::find(&console.database, &project.id, "office")
                .await
                .expect("read")
                .is_none()
        );
    }
}
