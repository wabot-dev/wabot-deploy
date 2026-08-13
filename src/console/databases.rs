//! Databases: the create form, the POST behind it, and the card the
//! service page shows instead of an image field.
//!
//! ## Why it is not the service form with a checkbox
//!
//! A managed database has no image to type, no environment to edit and
//! no tag to watch: the node writes all three. What it has instead is a
//! version, a size, and credentials it generated. Two forms that share
//! only the name field are two forms.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use super::auth::{back_with_error, field, read_form, see_other, signed_in, SessionMiddleware};
use super::language::t;
use super::shell::{Area, Frame};
use super::{layout, ConsoleState};
use crate::platform::databases::Database;
use crate::platform::{access, databases, postgres, presets, replicas};

#[derive(Debug, Deserialize, Validate)]
pub struct DatabaseForm {
    pub project: String,
    pub error: Option<String>,
}

#[injectable]
pub struct DatabasePages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl DatabasePages {
    #[view("/projects/:project/databases/new")]
    #[middleware(SessionMiddleware)]
    async fn new_database(&self, query: DatabaseForm) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        if !allowed.may_deploy() {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        }

        let action = format!("/projects/{}/databases", project.slug);
        let back = format!("/projects/{}", project.slug);
        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/databases/new", project.slug),
        )
        .allowing(allowed);

        layout::head("Create database");
        let body = super::language::scoped(account.language, || {
            rsx! {
            (layout::style_tag())
                <div class="stack-sm">
                    <h1>(t("Create database"))</h1>
                </div>
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                <form method="post" action=(&action) class="card stack">
                    <label for="name">(t("Name"))</label>
                    <input id="name" name="name" type="text" autocomplete="off" required autofocus>

                    <label for="version">(t("Version"))</label>
                    <select id="version" name="version">
                        @for version in postgres::VERSIONS {
                            @if version == postgres::DEFAULT_VERSION {
                                <option value=(version) selected>("PostgreSQL ")(version)</option>
                            } @else {
                                <option value=(version)>("PostgreSQL ")(version)</option>
                            }
                        }
                    </select>
                    <p class="field-hint">(t("The image is pulled from Docker Hub. The major version \
                         is fixed once the database exists: changing it is a data migration, not \
                         an image change."))</p>

                    <label for="memory">(t("Memory"))</label>
                    <select id="memory" name="memory">
                        @for rung in presets::LADDER {
                            @if rung == DEFAULT_PRESET {
                                <option value=(rung.to_string()) selected>(presets::label(rung))</option>
                            } @else {
                                <option value=(rung.to_string())>(presets::label(rung))</option>
                            }
                        }
                    </select>
                    <p class="field-hint">(t("A ceiling on the container and the engine's own \
                         settings, together. Postgres is given a quarter of it for shared buffers \
                         and told to expect half of it as cache — its defaults alone would be \
                         killed on the smaller sizes."))</p>
                    (sizes_table())

                    <div class="actions">
                        <button type="submit">(t("Create database"))</button>
                        <a class="btn btn-ghost" href=(&back)>(t("Cancel"))</a>
                    </div>
                </form>
        }
            .render()
            .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}

/// The rung offered first.
///
/// Not the smallest. 64 MB runs and it runs ten connections, which is
/// the right *floor* and the wrong default — somebody accepting the
/// suggestion should get a database that holds an application, not one
/// they have to come back to.
const DEFAULT_PRESET: u64 = 256 * 1024 * 1024;

/// What each size actually gives the engine.
///
/// On the form rather than in the documentation, because the number in
/// the selector is the container's ceiling and the numbers that decide
/// whether a query runs are these.
fn sizes_table() -> impl Renderable {
    rsx! {
        <table>
            <thead>
                <tr>
                    <th>(t("Memory"))</th>
                    <th>("shared_buffers")</th>
                    <th>("work_mem")</th>
                    <th>(t("Connections"))</th>
                </tr>
            </thead>
            <tbody>
                @for rung in presets::LADDER {
                    <tr>
                        <td>(presets::label(rung))</td>
                        <td class="mono">(format!("{} MB", postgres::tuning(rung).shared_buffers_mb))</td>
                        <td class="mono">(format!("{} MB", postgres::tuning(rung).work_mem_mb))</td>
                        <td class="mono">(postgres::tuning(rung).max_connections.to_string())</td>
                    </tr>
                }
            </tbody>
        </table>
    }
}

#[injectable]
pub struct DatabaseApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl DatabaseApi {
    /// Make a database, and start it.
    #[post("/projects/:project/databases")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some(project_slug) = super::services::project_slug(&path) else {
            return Ok(see_other("/"));
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, project_slug).await?
        else {
            return Ok(see_other("/?error=no+such+project"));
        };
        let here = format!("/projects/{}/databases/new", project.slug);
        if !allowed.may_deploy() {
            return Ok(see_other(&format!("/projects/{}", project.slug)));
        }

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let name = field(&form, "name").to_string();
        let version = field(&form, "version").to_string();
        let memory = match presets::parse(field(&form, "memory")) {
            Ok(Some(bytes)) => bytes,
            // Blank is not "no ceiling" here. An unbounded Postgres on
            // a node with one core is the node, so the form's empty
            // answer is the form's default answer.
            Ok(None) => DEFAULT_PRESET,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        let (service, _) =
            match databases::create(&self.state.database, &project.id, &name, &version, memory)
                .await
            {
                Ok(made) => made,
                Err(error) => return Ok(back_with_error(&here, &error.to_string())),
            };

        // Its own job on this node's own queue, which is the same
        // deployment the service page's button queues.
        let command = crate::deploy::jobs::DeployService {
            service_id: service.id.clone(),
            release_id: None,
        };
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            tracing::error!(%error, "could not queue the deployment");
        }

        Ok(see_other(&format!(
            "/projects/{}/services/{}",
            project.slug, service.slug
        )))
    }
}

/// What the service page shows for a managed database, in place of the
/// image and the environment nobody may edit.
///
/// The password is behind a `<details>`: the page is opened to read a
/// state badge far more often than to copy a credential, and a
/// connection string in plain sight is one that ends up in a
/// screenshot. It is markup rather than script, so it works with
/// scripting off like everything else here.
pub fn database_card<'a>(
    row: &'a Database,
    replicas: &'a [replicas::Replica],
    address: Option<String>,
    memory_limit: Option<u64>,
) -> impl Renderable + 'a {
    let url = address.as_ref().map(|address| {
        postgres::connection_url(
            &row.admin_user,
            &row.admin_password,
            address,
            postgres::PORT,
            &row.database_name,
        )
    });
    let copies = replicas.len();
    let standbys = replicas
        .iter()
        .filter(|replica| row.role_of(replica.slot) == postgres::Role::Standby)
        .count();

    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Database"))</p>
                <span class="who">
                    (row.engine.label())(" ")(&row.version)
                </span>
            </div>
            <dl class="kv">
                <dt>(t("Database name"))</dt>
                <dd class="mono">(&row.database_name)</dd>
                <dt>(t("User"))</dt>
                <dd class="mono">(&row.admin_user)</dd>
                <dt>(t("Memory"))</dt>
                <dd>(
                    memory_limit
                        .map(presets::label)
                        .unwrap_or_else(|| t("no ceiling").to_string())
                )</dd>
                <dt>(t("Copies"))</dt>
                <dd>(format!("{copies} ({standbys} read-only)"))</dd>
            </dl>
            @if let Some(url) = &url {
                <details>
                    <summary>(t("Connection string"))</summary>
                    <p class="mono">(url)</p>
                    <p class="field-hint">(t("Reachable from any container in this project. The \
                         address is reserved for this copy, so it survives a redeployment."))</p>
                </details>
            } @else {
                <p class="tile-detail">(t("It has no address yet. A connection string appears once \
                     it has been deployed."))</p>
            }
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use crate::platform::services;
    use wabot::rest::axum::http::StatusCode;

    async fn project(console: &Console) -> String {
        crate::platform::projects::create(&console.database, "shared")
            .await
            .expect("project")
            .id
    }

    /// The form somebody fills in, and everything it has to leave
    /// behind for the deploy path to start a database rather than an
    /// empty container that forgets.
    #[tokio::test]
    async fn the_form_makes_a_database_with_a_volume_a_port_and_a_size() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        project(&console).await;

        console
            .harness
            .post("/projects/shared/databases")
            .header("cookie", &cookie)
            .form(&[
                ("name", "Orders"),
                ("version", "17"),
                ("memory", &(128 * 1024 * 1024).to_string()),
            ])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let service = services::all(&console.database, None)
            .await
            .expect("services")
            .pop()
            .expect("one");
        assert_eq!(service.kind, services::Kind::Postgres);
        assert_eq!(service.image, "docker.io/library/postgres:17-alpine");
        assert_eq!(service.memory_limit, Some(128 * 1024 * 1024));

        let row = databases::of_service(&console.database, &service.id)
            .await
            .expect("query")
            .expect("an engine row");
        assert_eq!(row.database_name, "orders");
        assert!(!row.admin_password.is_empty());
    }

    /// A size nothing offers would be a page showing one number and a
    /// container running another.
    #[tokio::test]
    async fn a_size_off_the_ladder_comes_back_to_the_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        project(&console).await;

        let response = console
            .harness
            .post("/projects/shared/databases")
            .header("cookie", &cookie)
            .form(&[("name", "Orders"), ("version", "17"), ("memory", "100")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);

        assert!(
            services::all(&console.database, None)
                .await
                .expect("services")
                .is_empty(),
            "a refusal left a service behind"
        );
    }

    /// The node writes a managed database's environment, so the form
    /// that edits one must be refused even when the request did not
    /// come from the page that hides it.
    #[tokio::test]
    async fn the_environment_of_a_database_cannot_be_posted_at() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let project_id = project(&console).await;
        let (service, _) = databases::create(
            &console.database,
            &project_id,
            "orders",
            "17",
            64 * 1024 * 1024,
        )
        .await
        .expect("created");

        console
            .harness
            .post("/projects/shared/services/orders/env")
            .header("cookie", &cookie)
            .form(&[("env", "POSTGRES_PASSWORD=mine")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let stored = services::find(&console.database, &service.id)
            .await
            .expect("query")
            .expect("there");
        assert!(
            stored.env.is_empty(),
            "the node's own environment was overwritten from a form"
        );
    }
}
