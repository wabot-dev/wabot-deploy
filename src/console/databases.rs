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
    /// Where this database's certificate comes from.
    ///
    /// **ACME is refused unless both names resolve to this node**, and both
    /// is the point: the read pool's name is derived from the primary's, so
    /// an operator who added one DNS record and not the other would have a
    /// certificate order that cannot pass — twice a day, against an
    /// authority that locks the account at five failures an hour.
    ///
    /// The renewal loop makes the same check before it spends anything, so
    /// this is not what protects the account. It is what gives an answer
    /// *now*, in the form, instead of a silence somebody has to go and read
    /// the certificate page to understand.
    #[post("/projects/:project/services/:service/database/certificate")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn certificate(&self, request: Request) -> RestResult<Response> {
        // The service page's locator, because this is one of its forms: it
        // is the same permission question, the same refusal for a service
        // that arrived on an errand, and the same "back to here".
        let path = request.uri().path().to_string();
        let Some((project, service, here)) =
            super::services::locate_for(&self.state, &self.auth, &path).await?
        else {
            return Ok(see_other("/"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let names = crate::deploy::certificate_names(
            &self.state.database,
            &self.state.config,
            &project,
            &service,
        )
        .await;
        let Some(name) = names.first().cloned() else {
            return Ok(back_with_error(&here, "this database has no name yet"));
        };

        let renew_with = match super::nodes::source_from(&form, &name) {
            Ok(renew_with) => renew_with,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        if renew_with == crate::edge::policy::RenewWith::Acme {
            let Some(node_domain) =
                crate::node::settings::domain(&self.state.database, &self.state.config).await
            else {
                return Ok(back_with_error(
                    &here,
                    "this node has no domain of its own, so it cannot check whether those                      names point here — set one on the node page first",
                ));
            };
            // Both, and named individually when one fails: "it does not
            // resolve" about a name the operator never typed is a message
            // they cannot act on.
            for name in [name.clone(), crate::deploy::hosts::pool_name(&name)] {
                let outcome = crate::deploy::dns::resolves_here(&name, &node_domain).await;
                if !outcome.ok() {
                    return Ok(back_with_error(&here, &outcome.explain(&name)));
                }
            }
        }

        if let Err(error) =
            super::nodes::store_source(&self.state.database, &self.state.config, &name, &renew_with)
                .await
        {
            return Ok(back_with_error(&here, &error));
        }
        // The loop owns issuance; this only says there is something to look
        // at, so the answer arrives in seconds rather than at the next pass.
        self.state.certificates.now();

        Ok(see_other(&here))
    }

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
    // Every name it answers to, from `deploy::certificate_names` — the same
    // list the certificate is built from, so this page cannot offer a string
    // naming something the certificate does not cover.
    names: &'a [String],
    // Whether this node is the one signing. It decides a single parameter,
    // and getting it wrong is a string that fails: `sslrootcert` pointing at
    // an authority that did not sign is as broken as leaving it out when it
    // did.
    signs_here: bool,
) -> impl Renderable + 'a {
    // Only once it has an address, which is only once it has been
    // deployed. The names exist from the moment the database does — they
    // are derived from the project and the node's domain — so without this
    // the page would hand out a connection string for something that has
    // never run.
    //
    // The address itself is not shown here: every copy's is in the
    // placement table below, which is where somebody looking for one
    // looks.
    let strings = address
        .as_ref()
        .and_then(|_| Strings::of(row, names, signs_here));
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
            @if let Some(strings) = &strings {
                (connection_block(strings))
            } @else {
                <p class="tile-detail">(t("It has no address yet. A connection string appears once \
                     it has been deployed."))</p>
            }
        </section>
    }
}

/// The four strings this database can be reached by, all four rendered.
///
/// ## Why all four, and why the choosing is CSS
///
/// The page must work with scripting off, so the alternative to rendering
/// every string is a round trip to change a radio button. They are four
/// short lines: the markup holds them all and `:has()` shows the chosen
/// one. Nothing is fetched, nothing is built in the browser, and with no
/// JavaScript at all the first one is showing because its radio is checked.
struct Strings {
    /// `orders.db-test.<domain>` — unique across the network, and the only
    /// form a certificate authority could ever sign.
    primary_full: Option<String>,
    pool_full: Option<String>,
    /// `orders.db-test` — what a container in this project resolves. Shorter
    /// to read and meaningless anywhere else, which is the trade.
    primary_short: String,
    pool_short: String,
}

impl Strings {
    /// Built from the certificate's own name list, so every string offered
    /// is one `verify-full` will accept.
    ///
    /// `None` when the database has no name at all, which is a database that
    /// has never been deployed.
    fn of(row: &Database, names: &[String], signs_here: bool) -> Option<Self> {
        // `verify-full` always: encryption without identity is what
        // `require` buys, and every name here is on the certificate.
        //
        // The authority only when this node is the one that signed. With a
        // public one the client's own trust store is what checks it, and
        // naming a file that a laptop does not have would be a string that
        // fails for the one reader most likely to paste it.
        let suffix = match signs_here {
            true => "?sslmode=verify-full&sslrootcert=/etc/wabot/ca.crt",
            false => "?sslmode=verify-full",
        };
        let dsn = |host: &str| {
            format!(
                "{}{suffix}",
                postgres::connection_url(
                    &row.admin_user,
                    &row.admin_password,
                    host,
                    postgres::PORT,
                    &row.database_name,
                )
            )
        };
        // Longest first is how `certificate_names` orders them: the
        // qualified name, then the bare slug, then slug and project. So the
        // qualified pair is whatever holds a dot past the project, and the
        // short pair is the two-part one — chosen by shape rather than by
        // rebuilding the names here, which would be a second opinion about
        // what this database is called.
        let pool_mark = crate::deploy::hosts::READ_ONLY;
        let is_pool = |name: &&String| {
            name.split('.')
                .next()
                .is_some_and(|first| first.ends_with(pool_mark))
        };
        let parts = |name: &&String| name.split('.').count();

        let primary_short = names
            .iter()
            .filter(|name| !is_pool(name) && parts(name) == 2)
            .min_by_key(|name| name.len())?
            .clone();
        let pool_short = names
            .iter()
            .filter(|name| is_pool(name) && parts(name) == 2)
            .min_by_key(|name| name.len())?
            .clone();

        Some(Self {
            primary_full: names
                .iter()
                .find(|name| !is_pool(name) && parts(name) > 2)
                .map(|name| dsn(name)),
            pool_full: names
                .iter()
                .find(|name| is_pool(name) && parts(name) > 2)
                .map(|name| dsn(name)),
            primary_short: dsn(&primary_short),
            pool_short: dsn(&pool_short),
        })
    }
}

/// One string at a time, chosen without a round trip and without a script.
///
/// The string used to be a paragraph, so a browser broke it across lines
/// wherever it liked — and a connection string with a line break in it is a
/// connection string nobody can use. It is a `<pre>` that scrolls now.
///
/// The copy button is **revealed** by the `copy` island rather than
/// rendered: with scripting off a button that cannot copy is a control that
/// lies, and the text is selectable either way.
/// Where this database's certificate comes from, and how to verify it.
///
/// The same control a service's hostname has and the node's own — one
/// question asked in three places, so it is one form. What a database adds
/// is the other half: with the node signing, a client outside a container
/// has nothing to check against until it holds the authority, so the page
/// hands it over rather than describing it.
pub fn certificate_card<'a>(
    action: &'a str,
    policy: &'a crate::edge::policy::Policy,
) -> impl Renderable + 'a {
    let signs_here = matches!(
        policy.renew_with,
        crate::edge::policy::RenewWith::SelfSigned
    );
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("Certificate"))</p>
            (super::nodes::certificate_source_form(action, policy))
            @if signs_here {
                <p class="field-hint">(t("This node signs it, so a client has to be given the \
                     authority before it can verify anything. A container gets it at \
                     /etc/wabot/ca.crt; anything else needs the file."))</p>
                <div class="actions">
                    <a class="btn btn-secondary btn-sm" href="/ca.crt" download>
                        (t("Download the authority"))
                    </a>
                </div>
            } @else {
                <p class="field-hint">(t("A public authority signs it, so any client verifies it \
                     with the trust store it already has — the connection string needs no \
                     certificate of its own."))</p>
            }
        </section>
    }
}

/// One string, in a block that does not break it.
fn value<'a>(which: &'a str, dsn: &'a str, copy: &'a str, copied: &'a str) -> impl Renderable + 'a {
    rsx! {
        // The line is what CSS shows or hides, so the button the island
        // appends belongs to the string beside it rather than to the three
        // that are not on screen — and it sits next to the string instead
        // of on top of the end somebody is reading.
        <div class="dsn-line" data-dsn=(which)>
            <pre class="dsn-value" data-copy
                 data-copy-label=(copy) data-copied-label=(copied)>(dsn)</pre>
        </div>
    }
}

fn connection_block(strings: &Strings) -> impl Renderable + '_ {
    // The words the island puts on the button it makes. Passed as data
    // because the script has no language and the account has one — the same
    // rule the badge words follow.
    let (copy, copied) = (t("Copy"), t("Copied"));
    let inner = rsx! {
        <details>
            <summary>(t("Connection string"))</summary>
            <div class="dsn stack-sm">
                // Two groups, because they are two questions: which name,
                // and which copy. One row mixing them read as three
                // unrelated switches.
                <div class="dsn-pick">
                    <div class="dsn-group">
                        <label class="check">
                            <input type="radio" name="dsn-name" id="dsn-public" checked>
                            (t("Public"))
                        </label>
                        <label class="check">
                            <input type="radio" name="dsn-name" id="dsn-private">
                            (t("Private"))
                        </label>
                    </div>
                    <div class="dsn-group">
                        <label class="check">
                            <input type="radio" name="dsn-target" id="dsn-primary" checked>
                            (t("Primary"))
                        </label>
                        <label class="check">
                            <input type="radio" name="dsn-target" id="dsn-pool">
                            (t("Read pool"))
                        </label>
                    </div>
                </div>

                // Every string, with the words the island puts on the button
                // it makes: the script has no language and the account has
                // one, which is the rule the badge words follow too.
                <div class="dsn-values">
                    (value("primary-full", strings.primary_full.as_ref()
                        .unwrap_or(&strings.primary_short), copy, copied))
                    (value("pool-full", strings.pool_full.as_ref()
                        .unwrap_or(&strings.pool_short), copy, copied))
                    (value("primary-short", &strings.primary_short, copy, copied))
                    (value("pool-short", &strings.pool_short, copy, copied))
                </div>

                // One line, and it is the one that stops the labels lying:
                // "public" is the name's form, not a promise that the
                // internet can reach it.
                <p class="field-hint">(t("Both resolve inside this project, on any node holding \
                     a copy. Neither reaches the database from outside the node — that is a \
                     published port, which is not built."))</p>
            </div>
        </details>
    }
    .render()
    .into_inner();

    // Rendered first, then wrapped: `rsx!` expands to a closure that
    // captures by move, and nesting one inside the island's would have both
    // wanting the same borrows.
    wabot::ui::hypertext::island(
        "copy",
        &serde_json::json!({}),
        hypertext::Raw::dangerously_create(&inner),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use crate::platform::services;
    use wabot::rest::axum::http::StatusCode;

    /// The card names exactly what the certificate covers, because the
    /// names come from the function the certificate is built from.
    ///
    /// A page that named something else would be telling somebody to write
    /// a connection string `verify-full` rejects — which is worse than
    /// naming nothing, and naming nothing is what it did.
    #[tokio::test]
    async fn the_names_on_the_page_are_the_names_on_the_certificate() {
        let database = crate::db::open_in_memory().await.expect("open");
        crate::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("domain");
        let project = crate::platform::projects::create(&database, "db-test")
            .await
            .expect("project");
        let (service, row) = crate::platform::databases::create(
            &database,
            &project.id,
            "orders",
            "17",
            256 * 1024 * 1024,
        )
        .await
        .expect("created");

        let names = crate::deploy::certificate_names(
            &database,
            &crate::config::Config::default(),
            &project,
            &service,
        )
        .await;
        // The qualified name first, which is how `certificate_names` orders
        // them and what `Strings::of` reads the four choices out of.
        assert_eq!(
            names.first().map(String::as_str),
            Some("orders.db-test.node.example")
        );

        let replicas = crate::platform::replicas::of_service(&database, &service.id)
            .await
            .expect("replicas");
        let card = database_card(
            &row,
            &replicas,
            Some("10.42.2.200".into()),
            None,
            &names,
            true,
        )
        .render()
        .into_inner();
        // Four strings, all four in the markup, because choosing is CSS: the
        // page has to work with scripting off, and the alternative to
        // rendering every one is a round trip to move a radio button.
        for host in [
            "@orders.db-test.node.example:5432/orders",
            "@orders-ro.db-test.node.example:5432/orders",
            "@orders.db-test:5432/orders",
            "@orders-ro.db-test:5432/orders",
        ] {
            assert!(card.contains(host), "no string for {host}: {card}");
        }
        // And every one of them verifies, with the authority the node
        // places — a string offering `require` would be encryption without
        // identity, which is what this page used to hand out.
        // `&` arrives escaped, which is the markup being correct rather
        // than the string being wrong.
        // `&` arrives escaped, which is the markup being correct rather
        // than the string being wrong.
        assert_eq!(
            card.matches("sslmode=verify-full&amp;sslrootcert=/etc/wabot/ca.crt")
                .count(),
            4,
            "one per choice: {card}"
        );

        // The ids the stylesheet selects on, and the closed section.
        //
        // A half-applied edit once shipped the CSS for these without the
        // markup, so every rule matched nothing and the page showed no
        // string at all — which looked, from outside, exactly like a
        // deployment that had not happened.
        for id in ["dsn-public", "dsn-private", "dsn-primary", "dsn-pool"] {
            assert!(card.contains(&format!("id=\"{id}\"")), "no {id}: {card}");
        }
        assert!(
            card.contains("<details>"),
            "the section is closed until somebody opens it: {card}"
        );
    }

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
