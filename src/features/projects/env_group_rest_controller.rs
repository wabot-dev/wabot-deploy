use std::collections::BTreeMap;
use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::domain::authz::node_user;
use crate::domain::values::slug::Slug;
use crate::features::projects::project_rest_controller::locate_for;
use crate::features::projects::project_settings_tabs_component::ProjectSettingsTab;
use crate::shell::auth::{
    back_with_error, field, parse_env, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

#[injectable]
pub struct EnvGroupRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl EnvGroupRestController {
    #[post("/projects/:project/env-groups")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, _)) = locate_for(&self.state, &self.auth, &path).await? else {
            return Ok(see_other("/projects?error=no+such+project"));
        };
        let here = ProjectSettingsTab::Environment.path(&project.slug);
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let env = match parse_env(field(&form, "env")) {
            Ok(pairs) => pairs.into_iter().collect::<BTreeMap<_, _>>(),
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        let asked = crate::features::projects::create_env_group_request::CreateEnvGroupRequest {
            name: Slug::of(field(&form, "name")),
            env,
        };
        let made = match node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_create_env_group::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_create_env_group::run(
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
            Ok(()) => Ok(see_other(&format!("{here}#group-{}", asked.name))),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/env-groups/:group")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn save(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, segments)) = locate_for(&self.state, &self.auth, &path).await?
        else {
            return Ok(see_other("/projects?error=no+such+project"));
        };
        let here = ProjectSettingsTab::Environment.path(&project.slug);
        let name = segments.get(3).copied().unwrap_or_default().to_string();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked = match parse_env(field(&form, "env")) {
            Ok(pairs) => crate::features::projects::set_env_group_request::SetEnvGroupRequest {
                env: pairs.into_iter().collect(),
            },
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        let saved = match node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_set_env_group::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    &name,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_set_env_group::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    &name,
                    &asked,
                )
                .await
            }
        };
        let saved = match saved {
            Ok(saved) => saved,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        for redeploy in &saved.redeploy {
            let command = crate::infra::deploy::jobs::DeployService::new(
                &redeploy.service_id,
                redeploy.release_id.as_deref(),
            );
            if let Err(error) =
                wabot::async_jobs::run_command(&self.state.container, &command).await
            {
                tracing::error!(service_id = %redeploy.service_id, %error, "could not queue a deployment");
            }
        }
        Ok(see_other(&format!("{here}#group-{name}")))
    }

    #[post("/projects/:project/env-groups/:group/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, allowed, segments)) = locate_for(&self.state, &self.auth, &path).await?
        else {
            return Ok(see_other("/projects?error=no+such+project"));
        };
        let here = ProjectSettingsTab::Environment.path(&project.slug);
        let name = segments.get(3).copied().unwrap_or_default().to_string();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let deleted = match node_user::belongs_here(&account) {
            true => {
                crate::features::projects::node_user_delete_env_group::run(
                    &self.state.database,
                    &account,
                    &project,
                    &allowed,
                    &name,
                )
                .await
            }
            false => {
                crate::features::projects::tenant_user_delete_env_group::run(
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
    use crate::domain::values::roles::ProjectRole;
    use crate::infra::repositories::{access, env_groups, projects, services};
    use crate::web::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    async fn shop(console: &Console, cookie: &str, slug: &str) -> (Project, Service, Service) {
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
        let mut made = Vec::new();
        for name in ["orders", "ledger"] {
            made.push(
                services::create(
                    &console.database,
                    &project.id,
                    name,
                    "hub.example.com/shop/app@sha256:abc",
                    &[],
                )
                .await
                .expect("service"),
            );
        }
        let ledger = made.pop().expect("ledger");
        let orders = made.pop().expect("orders");
        (project, orders, ledger)
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

    async fn join(console: &Console, cookie: &str, service: &str, groups: &[&str]) -> String {
        let names: Vec<String> = groups.iter().map(|id| format!("group-{id}")).collect();
        let form: Vec<(&str, &str)> = names.iter().map(|name| (name.as_str(), "1")).collect();
        post(
            console,
            cookie,
            &format!("/projects/shop/services/{service}/env-groups"),
            &form,
        )
        .await
    }

    async fn variable(console: &Console, service: &Service, key: &str) -> Option<String> {
        let fresh = services::find(&console.database, &service.id)
            .await
            .expect("read")
            .expect("there");
        env_groups::environment(&console.database, &fresh)
            .await
            .expect("read")
            .get(key)
            .cloned()
    }

    #[tokio::test]
    async fn a_group_saved_once_reaches_every_service_that_joined_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let (project, orders, ledger) = shop(&console, &cookie, "shop").await;

        post(
            &console,
            &cookie,
            "/projects/shop/env-groups",
            &[
                ("name", "smtp"),
                ("env", "SMTP_HOST=mail\nSMTP_USER=shared"),
            ],
        )
        .await;
        let smtp = env_groups::find(&console.database, &project.id, "smtp")
            .await
            .expect("read")
            .expect("made");
        for service in ["orders", "ledger"] {
            let back = join(&console, &cookie, service, &[&smtp.id]).await;
            assert!(!back.contains("error="), "{back}");
        }
        assert_eq!(
            variable(&console, &orders, "SMTP_HOST").await.as_deref(),
            Some("mail")
        );

        let back = post(
            &console,
            &cookie,
            "/projects/shop/env-groups/smtp",
            &[("env", "SMTP_HOST=mail.eu\nSMTP_USER=shared")],
        )
        .await;
        assert!(!back.contains("error="), "{back}");
        for service in [&orders, &ledger] {
            assert_eq!(
                variable(&console, service, "SMTP_HOST").await.as_deref(),
                Some("mail.eu")
            );
        }

        let page = console
            .harness
            .get("/projects/shop/settings/environment")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(page.contains("SMTP_HOST=mail.eu"), "{page}");
        assert!(
            page.contains("ledger, orders") || page.contains("orders, ledger"),
            "{page}"
        );
        let tab = console
            .harness
            .get("/projects/shop/services/orders/settings/environment")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            tab.contains(&format!(r#"name="group-{}" value="1" checked"#, smtp.id)),
            "{tab}"
        );
    }

    /// Two groups that set one variable would leave which value a
    /// container got to the order they were read in.
    #[tokio::test]
    async fn no_service_ends_up_with_two_groups_that_set_one_variable() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let (project, orders, _) = shop(&console, &cookie, "shop").await;
        for (name, env) in [
            ("smtp", "SMTP_HOST=mail"),
            ("smtp-eu", "SMTP_HOST=mail.eu"),
            ("sentry", "SENTRY_DSN=https://key@sentry"),
        ] {
            post(
                &console,
                &cookie,
                "/projects/shop/env-groups",
                &[("name", name), ("env", env)],
            )
            .await;
        }
        let id = |name: &'static str| {
            let (console, project) = (&console, &project);
            async move {
                env_groups::find(&console.database, &project.id, name)
                    .await
                    .expect("read")
                    .expect("made")
                    .id
            }
        };
        let (smtp, smtp_eu, sentry) = (id("smtp").await, id("smtp-eu").await, id("sentry").await);

        let back = join(&console, &cookie, "orders", &[&smtp, &smtp_eu]).await;
        assert!(back.contains("error="), "{back}");
        assert!(env_groups::of_service(&console.database, &orders.id)
            .await
            .expect("read")
            .is_empty());

        join(&console, &cookie, "orders", &[&smtp, &sentry]).await;
        let back = post(
            &console,
            &cookie,
            "/projects/shop/env-groups/sentry",
            &[("env", "SENTRY_DSN=https://key@sentry\nSMTP_HOST=elsewhere")],
        )
        .await;
        assert!(back.contains("error="), "{back}");
        assert_eq!(
            variable(&console, &orders, "SMTP_HOST").await.as_deref(),
            Some("mail")
        );
    }

    /// A group id is the only thing the form names, so the check that it
    /// is this project's is the one that keeps another project's secrets
    /// out of this service.
    #[tokio::test]
    async fn a_service_joins_only_its_own_projects_groups() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let (_, orders, _) = shop(&console, &cookie, "shop").await;
        let (other, _, _) = shop(&console, &cookie, "other").await;
        post(
            &console,
            &cookie,
            "/projects/other/env-groups",
            &[("name", "secrets"), ("env", "TOKEN=theirs")],
        )
        .await;
        let theirs = env_groups::find(&console.database, &other.id, "secrets")
            .await
            .expect("read")
            .expect("made");

        let back = join(&console, &cookie, "orders", &[&theirs.id]).await;

        assert!(back.contains("error="), "{back}");
        assert_eq!(variable(&console, &orders, "TOKEN").await, None);
    }

    #[tokio::test]
    async fn a_group_somebody_joined_is_not_deleted() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let (project, _, _) = shop(&console, &cookie, "shop").await;
        post(
            &console,
            &cookie,
            "/projects/shop/env-groups",
            &[("name", "smtp"), ("env", "SMTP_HOST=mail")],
        )
        .await;
        let smtp = env_groups::find(&console.database, &project.id, "smtp")
            .await
            .expect("read")
            .expect("made");
        join(&console, &cookie, "orders", &[&smtp.id]).await;

        let back = post(
            &console,
            &cookie,
            "/projects/shop/env-groups/smtp/delete",
            &[],
        )
        .await;
        assert!(back.contains("error="), "{back}");
        assert!(env_groups::find(&console.database, &project.id, "smtp")
            .await
            .expect("read")
            .is_some());

        join(&console, &cookie, "orders", &[]).await;
        post(
            &console,
            &cookie,
            "/projects/shop/env-groups/smtp/delete",
            &[],
        )
        .await;
        assert!(env_groups::find(&console.database, &project.id, "smtp")
            .await
            .expect("read")
            .is_none());
    }

    #[tokio::test]
    async fn a_viewer_neither_sees_nor_changes_a_group() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let (project, _, _) = shop(&console, &admin, "shop").await;
        let viewer = console.joined_as(&admin, "viewer").await;
        let person = crate::infra::accounts::all(&console.database)
            .await
            .expect("query")
            .into_iter()
            .find(|account| account.username == "viewer")
            .expect("joined");
        access::grant(
            &console.database,
            &person.id,
            &project.id,
            ProjectRole::Viewer,
        )
        .await
        .expect("granted");

        let page = console
            .harness
            .get("/projects/shop/settings/environment")
            .header("cookie", &viewer)
            .send()
            .await;
        page.assert_status(StatusCode::FOUND);

        post(
            &console,
            &viewer,
            "/projects/shop/env-groups",
            &[("name", "smtp"), ("env", "SMTP_HOST=mail")],
        )
        .await;
        assert!(env_groups::of_project(&console.database, &project.id)
            .await
            .expect("read")
            .is_empty());
    }
}
