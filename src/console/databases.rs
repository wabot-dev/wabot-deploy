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

/// Where these three forms live, and so where saving one goes back to.
///
/// The name, the certificate and the published port are settings, and
/// `locate_for` answers with the *project* — the right fallback for a
/// control on the project's own list, and the wrong place to land
/// somebody who was half-way down a settings page. A save that moves
/// the page is a save that hides what it did.
fn settings_of(
    project: &crate::platform::projects::Project,
    service: &crate::platform::services::Service,
) -> String {
    format!(
        "/projects/{}/services/{}/settings",
        project.slug, service.slug
    )
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
    /// What this database is called.
    ///
    /// The name is written and everything derived from it follows in the
    /// same request: the certificate is reissued under the new name and
    /// handed to the running servers, and the project's containers get
    /// their `/etc/hosts` rewritten. A rename that left either behind would
    /// be a page saying one name while the database answered to another.
    ///
    /// No DNS check here, deliberately. A name signed by this node needs to
    /// resolve nowhere but inside the project — that is the ordinary case —
    /// and the check belongs where the answer matters, which is the moment
    /// somebody asks a public authority for it.
    #[post("/projects/:project/services/:service/database/name")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn name(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) =
            super::services::locate_for(&self.state, &self.auth, &path).await?
        else {
            return Ok(see_other("/"));
        };
        let here = settings_of(&project, &service);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let typed = crate::platform::ports::normalize_hostname(field(&form, "name"));
        if typed.is_empty() {
            return Ok(back_with_error(
                &here,
                "a database is reached by name, so it has to have one",
            ));
        }

        let was = crate::deploy::certificate_names(
            &self.state.database,
            &self.state.config,
            &project,
            &service,
        )
        .await
        .into_iter()
        .next();

        let Some(port) = crate::platform::ports::of_service(&self.state.database, &service.id)
            .await?
            .into_iter()
            .next()
        else {
            return Ok(back_with_error(&here, "this database has no port row"));
        };
        if let Err(error) =
            crate::platform::ports::set_hostname(&self.state.database, &port.id, Some(&typed)).await
        {
            return Ok(back_with_error(&here, &error.to_string()));
        }

        // The choice of certificate follows the name it was made about. A
        // policy left on the old one would be an answer nothing reads and a
        // new name quietly back on the default.
        if let Some(was) = was.filter(|was| was != &typed) {
            let policy =
                crate::edge::policy::for_name(&self.state.database, &self.state.config, &was).await;
            if let Err(error) = super::nodes::store_source(
                &self.state.database,
                &self.state.config,
                &typed,
                &policy.renew_with,
            )
            .await
            {
                return Ok(back_with_error(&here, &error));
            }
            if let Err(error) = crate::edge::policy::clear(&self.state.database, &was).await {
                tracing::warn!(%was, %error, "could not forget the old name's certificate source");
            }
        }

        // Issued under the new name and handed to whatever is running,
        // which is what `refresh_certificates` does on its own timer — the
        // rename only makes it worth doing now.
        if let Err(error) = self.state.deployer.refresh_certificates().await {
            tracing::warn!(%error, "could not reissue after the rename");
        }
        // And the names inside the containers, which is where this name is
        // resolved from.
        self.state.deployer.sync_routes().await;
        self.state.certificates.now();

        Ok(see_other(&here))
    }

    /// Open a port of this node's onto the database, or close it.
    ///
    /// Which is the only way anything outside the node reaches it: a
    /// certificate makes the name verifiable, and this is what makes the
    /// database answer. The two are separate because they fail separately
    /// — a name with a good certificate and no port resolves to a machine
    /// that says nothing.
    ///
    /// The primary only. A pool that answered from outside would need a
    /// port per replica and something choosing between them, which is a
    /// load balancer and has its own justification rather than being a
    /// detail of this.
    #[post("/projects/:project/services/:service/database/publish")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn publish(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) =
            super::services::locate_for(&self.state, &self.auth, &path).await?
        else {
            return Ok(see_other("/"));
        };
        let here = settings_of(&project, &service);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let Some(port) = crate::platform::ports::of_service(&self.state.database, &service.id)
            .await?
            .into_iter()
            .next()
        else {
            return Ok(back_with_error(&here, "this database has no port row"));
        };

        let publish = field(&form, "publish") == "on";
        if let Err(error) =
            crate::platform::ports::set_published(&self.state.database, &port.id, publish).await
        {
            return Ok(back_with_error(&here, &error.to_string()));
        }

        // Redeployed, because a published port is an iptables rule made
        // when the container joins the network — not something that can be
        // added to one already running.
        let command = crate::deploy::jobs::DeployService {
            service_id: service.id.clone(),
            release_id: None,
        };
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            tracing::error!(service = %service.id, %error, "could not queue a deployment");
        }

        Ok(see_other(&here))
    }

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
        let Some((project, service, _)) =
            super::services::locate_for(&self.state, &self.auth, &path).await?
        else {
            return Ok(see_other("/"));
        };
        let here = settings_of(&project, &service);
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
    // The port the world reaches it on, when there is one. The strings
    // above are the project's own; this is what makes the sentence under
    // them true rather than a description of the product at some past
    // moment.
    published: Option<u16>,
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
                (connection_block(strings, published))
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
    /// The long name — `orders.db-test.<domain>` — and the short one,
    /// `orders.db-test`. Both resolve in every container of the project;
    /// the long one also resolves in the world's DNS, which is what makes
    /// it the only form an authority could ever sign.
    primary: Vec<Choice>,
    pool: Vec<Choice>,
}

/// One string, and how to spell the name it uses.
struct Choice {
    /// `full` or `short` — what the radio selects and the stylesheet
    /// matches on.
    which: &'static str,
    label: &'static str,
    dsn: String,
}

impl Strings {
    /// Built from the certificate's own name list, so every string offered
    /// is one `verify-full` will accept.
    ///
    /// `None` when the database has no name at all, which is a database that
    /// has never been deployed.
    /// **Only what actually verifies.**
    ///
    /// A page that offered a string `verify-full` rejects would be worse
    /// than one that offered none, and there are two ways to produce one:
    ///
    /// - With a public authority, the short names are not on the
    ///   certificate and never can be — nothing outside this node resolves
    ///   them, so there is no challenge for an authority to set. Offering
    ///   the short form there is offering a string that fails.
    /// - On a node with no domain there **is** no long name: nothing to
    ///   derive one from, so the short one is not a convenience, it is the
    ///   only name the database has.
    ///
    /// So the name is a choice only when both spellings work, which is a
    /// node with a domain whose certificate it signs itself. Otherwise
    /// there is one spelling and no group to show.
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

        let long_of = |pool: bool| {
            names
                .iter()
                .find(|name| is_pool(name) == pool && parts(name) > 2)
                .map(|name| dsn(name))
        };
        let offer = |long: Option<String>, short: String| {
            let mut choices = Vec::new();
            if let Some(long) = long {
                choices.push(Choice {
                    which: "full",
                    label: "Long name",
                    dsn: long,
                });
            }
            // The short one only where it is on the certificate, which is
            // where this node signed it.
            if signs_here || choices.is_empty() {
                choices.push(Choice {
                    which: "short",
                    label: "Short name",
                    dsn: short,
                });
            }
            choices
        };

        Some(Self {
            primary: offer(long_of(false), dsn(&primary_short)),
            pool: offer(long_of(true), dsn(&pool_short)),
        })
    }

    /// Whether the name is a choice at all.
    fn names_differ(&self) -> bool {
        self.primary.len() > 1
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
/// What this database is called.
///
/// **A database always has a name**, unlike a service: a name is how it is
/// reached and how its certificate is verified, and there is no version of
/// this that works without one. So the field is never empty — it starts at
/// what every database had before a name could be chosen,
/// `<service>.<project>.<the node's domain>`, and the operator may put
/// anything there, including a domain that has nothing to do with this
/// machine's.
///
/// The read pool's name is not asked for. It is the primary's with `-ro` in
/// the first label, so one name governs both and there is no second thing
/// to keep in step — see `hosts::pool_name`.
pub fn name_card<'a>(action: &'a str, name: &'a str, pool: &'a str) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <p class="card-label">(t("Name"))</p>
            <form method="post" action=(action) class="stack">
                <input id="db-name" name="name" type="text" autocomplete="off"
                       class="mono" value=(name) required>
                <p class="field-hint">
                    (t("The read pool answers at "))<span class="mono">(pool)</span>
                    (t(" — the same name with -ro in its first label. Changing this \
                         reissues the certificate and rewrites the names inside every \
                         container of this project."))
                </p>
                <div class="actions">
                    <button class="btn btn-secondary" type="submit">(t("Save name"))</button>
                </div>
            </form>
        </section>
    }
}

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
    // What the node holds for this name right now. A source without a state
    // is a page saying what was *asked for* and never what happened —
    // which is what sent Jorge to `doctor` to find out whether his
    // certificate had been issued.
    cells: &'a super::nodes::CertificateCells,
) -> impl Renderable + 'a {
    let signs_here = matches!(
        policy.renew_with,
        crate::edge::policy::RenewWith::SelfSigned
    );
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("Certificate"))</p>
                <span class=(cells.badge)>
                    <span class=(cells.dot)></span>(t(cells.word))
                </span>
            </div>

            // The same three facts a service's name shows, because it is
            // the same question about a different name.
            <dl class="kv">
                <dt>(t("Issuer"))</dt>
                <dd>(&cells.issuer)</dd>
                <dt>(t("Renews in"))</dt>
                <dd>(&cells.renews)</dd>
            </dl>
            @if !cells.failure.is_empty() {
                <p class="failure">(&cells.failure)</p>
            }

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

/// Whether the world can reach this database, and on what port.
///
/// **The number is the node's to choose.** Two databases on one machine
/// cannot share a host port, and asking for one would be asking somebody
/// to remember what every other service on the node already took — the
/// allocator picks the lowest free one out of the node's range and the
/// unique index is what makes that safe.
///
/// Separate from the certificate, because they answer different questions
/// and Jorge named the difference: a certificate makes a name *verifiable*
/// and a port makes the database *reachable*. Until this is on, the name
/// resolves from outside and nothing answers.
pub fn published_card<'a>(
    action: &'a str,
    host_port: Option<u16>,
    name: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <section class="card stack">
            <div class="split">
                <p class="card-label">(t("From outside the node"))</p>
                @if host_port.is_some() {
                    <span class="badge badge-success">
                        <span class="dot dot-success"></span>(t("Published"))
                    </span>
                } @else {
                    <span class="badge">(t("Not published"))</span>
                }
            </div>

            @if let Some(port) = host_port {
                <dl class="kv">
                    <dt>(t("Reachable at"))</dt>
                    <dd class="mono">(name)(":")(port.to_string())</dd>
                </dl>
                <p class="field-hint">(t("The node chose the port, so it cannot collide with \
                     another database's. It survives a redeployment and changes only if you \
                     turn this off and on again."))</p>
            } @else {
                <p class="field-hint">(t("The name already resolves from outside and nothing \
                     answers there. Publishing opens a port on every interface of this node \
                     and maps it to the primary."))</p>
            }

            <form method="post" action=(action)>
                @if host_port.is_some() {
                    <input type="hidden" name="publish" value="off">
                    <button class="btn btn-ghost destructive" type="submit">
                        (t("Stop publishing"))
                    </button>
                } @else {
                    <input type="hidden" name="publish" value="on">
                    <button class="btn btn-secondary" type="submit">(t("Publish"))</button>
                }
            </form>
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

fn connection_block(strings: &Strings, published: Option<u16>) -> impl Renderable + '_ {
    // The words the island puts on the button it makes. Passed as data
    // because the script has no language and the account has one — the same
    // rule the badge words follow.
    let (copy, copied) = (t("Copy"), t("Copied"));
    let inner = rsx! {
        <details>
            <summary>(t("Connection string"))</summary>
            <div class="dsn stack-sm">
                // Two groups when there are two questions, one when the
                // name is not a choice — a node with no domain has only
                // the short name, and a public authority can only ever
                // sign the long one.
                <div class="dsn-pick">
                    @if strings.names_differ() {
                        <div class="dsn-group">
                            @for (index, choice) in strings.primary.iter().enumerate() {
                                <label class="check">
                                    @if index == 0 {
                                        <input type="radio" name="dsn-name"
                                               id=(format!("dsn-{}", choice.which)) checked>
                                    } @else {
                                        <input type="radio" name="dsn-name"
                                               id=(format!("dsn-{}", choice.which))>
                                    }
                                    (t(choice.label))
                                </label>
                            }
                        </div>
                    }
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

                // Every string that verifies, and the stylesheet shows the
                // chosen one. With no name group there is one spelling, so
                // its radio is the only thing selecting.
                <div class="dsn-values">
                    @for choice in &strings.primary {
                        (value(&format!("primary-{}", choice.which), &choice.dsn, copy, copied))
                    }
                    @for choice in &strings.pool {
                        (value(&format!("pool-{}", choice.which), &choice.dsn, copy, copied))
                    }
                </div>

                // The one line that stops a name being read as a promise —
                // and it has to know whether the port is open, because it
                // said "publishing is not built" for as long as that was
                // true and went on saying it after Jorge published one.
                // A hint that describes the product rather than this
                // database is a hint that goes stale on its own.
                @if let Some(port) = published {
                    <p class="field-hint">
                        (t("These reach it from inside the project, on any node holding a \
                             copy. From outside the node it answers on the published port, \
                             "))<span class="mono">(port.to_string())</span>(t(" — same name, \
                             same certificate."))
                    </p>
                } @else {
                    <p class="field-hint">(t("These reach it from inside the project, on any \
                         node holding a copy — the long name resolves in the world's DNS too, \
                         and nothing answers there until a port is published below."))</p>
                }
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
            None,
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
        // identity, which is what this page used to hand out. `&` arrives
        // escaped, which is the markup being right rather than the string
        // being wrong.
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
        for id in ["dsn-full", "dsn-short", "dsn-primary", "dsn-pool"] {
            assert!(card.contains(&format!("id=\"{id}\"")), "no {id}: {card}");
        }
        assert!(
            card.contains("<details>"),
            "the section is closed until somebody opens it: {card}"
        );
    }

    /// The page never offers a string `verify-full` would reject, and
    /// there are two ways to produce one.
    ///
    /// A public authority cannot sign the short names: nothing outside this
    /// node resolves them, so there is no challenge for it to set. And a
    /// node with no domain has no long name at all — there is nothing to
    /// derive one from, which makes the short one the only name the
    /// database has rather than a convenience that could be dropped.
    ///
    /// Jorge asked both questions in the same breath, and they pull in
    /// opposite directions: the first says offer fewer, the second says
    /// never offer none.
    #[tokio::test]
    async fn only_the_spellings_that_verify_are_offered() {
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
        let replicas = crate::platform::replicas::of_service(&database, &service.id)
            .await
            .expect("replicas");
        let address = Some("10.42.2.200".to_string());

        let public = database_card(&row, &replicas, address.clone(), None, &names, false, None)
            .render()
            .into_inner();
        assert!(public.contains("@orders.db-test.node.example:5432/orders"));
        assert!(
            !public.contains("@orders.db-test:5432/orders"),
            "a short name is not on a public certificate: {public}"
        );
        assert!(
            !public.contains("sslrootcert"),
            "and nothing has to be distributed to verify one: {public}"
        );
        assert!(
            !public.contains(r#"id="dsn-short""#),
            "with one spelling there is no name to choose: {public}"
        );

        // A node with no domain: the derivation has nothing to work from,
        // so only the short names exist.
        let short_only: Vec<String> = names
            .iter()
            .filter(|name| !name.ends_with("node.example"))
            .cloned()
            .collect();
        let bare = database_card(&row, &replicas, address, None, &short_only, true, None)
            .render()
            .into_inner();
        assert!(
            bare.contains("@orders.db-test:5432/orders"),
            "the only name it has: {bare}"
        );
        assert!(
            !bare.contains(r#"id="dsn-full""#),
            "and nothing to choose between: {bare}"
        );
    }

    /// A chosen name is what the database is called, everywhere.
    ///
    /// The certificate is stored under the first of these names and the
    /// project's containers resolve them, so a rename that reached one and
    /// not the other would be a client dialling a name no certificate holds
    /// — which is the failure `0032` was written about, from the other
    /// direction.
    #[tokio::test]
    async fn a_chosen_name_replaces_the_derived_one_everywhere() {
        let database = crate::db::open_in_memory().await.expect("open");
        crate::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("domain");
        let project = crate::platform::projects::create(&database, "db-test")
            .await
            .expect("project");
        let (service, _) = crate::platform::databases::create(
            &database,
            &project.id,
            "orders",
            "17",
            256 * 1024 * 1024,
        )
        .await
        .expect("created");
        let config = crate::config::Config::default();

        let derived =
            crate::deploy::certificate_names(&database, &config, &project, &service).await;
        assert_eq!(
            derived.first().map(String::as_str),
            Some("orders.db-test.node.example"),
            "what every database had before a name could be chosen"
        );

        let port = crate::platform::ports::of_service(&database, &service.id)
            .await
            .expect("ports")
            .pop()
            .expect("a database is created with one");
        crate::platform::ports::set_hostname(&database, &port.id, Some("db.example.com"))
            .await
            .expect("named");

        let chosen = crate::deploy::certificate_names(&database, &config, &project, &service).await;
        assert_eq!(
            chosen.first().map(String::as_str),
            Some("db.example.com"),
            "the row wins over the derivation"
        );
        assert!(
            chosen.contains(&"db-ro.example.com".to_string()),
            "and the pool follows it: {chosen:?}"
        );
        assert!(
            !chosen.iter().any(|name| name.ends_with("node.example")),
            "nothing keeps answering to the old name: {chosen:?}"
        );
        // The short names inside the project are untouched: they are what a
        // container beside it resolves and they cost nothing.
        assert!(chosen.contains(&"orders".to_string()));
        assert!(chosen.contains(&"orders.db-test".to_string()));
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
