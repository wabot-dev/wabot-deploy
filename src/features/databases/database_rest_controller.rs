//! What a managed database's forms post to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::console::auth::{
    back_with_error, field, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::console::ConsoleState;

#[injectable]
pub struct DatabaseRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl DatabaseRestController {
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
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some((project, service, _)) =
            crate::features::services::service_rest_controller::locate_for(
                &self.state,
                &self.auth,
                &path,
            )
            .await?
        else {
            return Ok(see_other("/"));
        };
        let here = settings_of(&project, &service);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let renamed = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::databases::node_user_name_database::run(
                    &self.state.database,
                    &self.state.config,
                    &account,
                    &project,
                    &service,
                    field(&form, "name"),
                )
                .await
            }
            false => {
                crate::features::databases::tenant_user_name_database::run(
                    &self.state.database,
                    &self.state.config,
                    &account,
                    &project,
                    &service,
                    field(&form, "name"),
                )
                .await
            }
        };
        let named = match renamed {
            Ok(named) => named,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        // The choice of certificate follows the name it was made about. A
        // policy left on the old one would be an answer nothing reads and a
        // new name quietly back on the default.
        if let Some(was) = named.was {
            let policy =
                crate::edge::policy::for_name(&self.state.database, &self.state.config, &was).await;
            if let Err(error) = crate::commands::store_certificate_source::run(
                &self.state.database,
                &self.state.config,
                &named.now,
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

        if let Err(error) = self.state.deployer.refresh_certificates().await {
            tracing::warn!(%error, "could not reissue after the rename");
        }
        self.state.deployer.sync_routes().await;
        self.state.certificates.now();

        Ok(see_other(&format!("{here}#name")))
    }

    /// Open a port of this node's onto the database, or close it.
    ///
    /// Which is the only way anything outside the node reaches it: a
    /// certificate makes the name verifiable, and this is what makes the
    /// database answer. The two are separate because they fail separately
    /// — a name with a good certificate and no port resolves to a machine
    /// that says nothing.
    #[post("/projects/:project/services/:service/database/publish")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn publish(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some((project, service, _)) =
            crate::features::services::service_rest_controller::locate_for(
                &self.state,
                &self.auth,
                &path,
            )
            .await?
        else {
            return Ok(see_other("/"));
        };
        let here = settings_of(&project, &service);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let published = field(&form, "publish") == "on";
        let done = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::databases::node_user_publish_database::run(
                    &self.state.database,
                    &account,
                    &service,
                    published,
                )
                .await
            }
            false => {
                crate::features::databases::tenant_user_publish_database::run(
                    &self.state.database,
                    &account,
                    &service,
                    published,
                )
                .await
            }
        };
        if let Err(reason) = done {
            return Ok(back_with_error(&here, &reason));
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

        Ok(see_other(&format!("{here}#published")))
    }

    /// Where this database's certificate comes from.
    #[post("/projects/:project/services/:service/database/certificate")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn certificate(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some((project, service, _)) =
            crate::features::services::service_rest_controller::locate_for(
                &self.state,
                &self.auth,
                &path,
            )
            .await?
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
        let Some(first) = names.first().cloned() else {
            return Ok(back_with_error(&here, "this database has no name yet"));
        };
        let renew_with = match crate::console::auth::source_from(&form, &first) {
            Ok(renew_with) => renew_with,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        let checked = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::databases::node_user_set_database_certificate::run(
                    &self.state.database,
                    &self.state.config,
                    &account,
                    &project,
                    &service,
                    &renew_with,
                )
                .await
            }
            false => {
                crate::features::databases::tenant_user_set_database_certificate::run(
                    &self.state.database,
                    &self.state.config,
                    &account,
                    &project,
                    &service,
                    &renew_with,
                )
                .await
            }
        };
        let name = match checked {
            Ok(name) => name,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        if let Err(error) = crate::commands::store_certificate_source::run(
            &self.state.database,
            &self.state.config,
            &name,
            &renew_with,
        )
        .await
        {
            return Ok(back_with_error(&here, &error));
        }
        // The loop owns issuance; this only says there is something to look
        // at, so the answer arrives in seconds rather than at the next pass.
        self.state.certificates.now();

        Ok(see_other(&format!("{here}#certificate")))
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
        let Some(project_slug) = crate::console::auth::project_slug(&path) else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{project_slug}/databases/new");
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // Which console this arrived at decides whose use case runs.
        // The two are the same today and will not stay that way: an
        // organisation has a quota and the machine's own frontier does
        // not.
        let asked = crate::features::databases::create_database_request::CreateDatabaseRequest {
            project: project_slug.to_string(),
            name: field(&form, "name").to_string(),
            version: field(&form, "version").to_string(),
            memory: field(&form, "memory").to_string(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }
        let made = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::databases::node_user_create_database::run(
                    &self.state.database,
                    &account,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::databases::tenant_user_create_database::run(
                    &self.state.database,
                    &account,
                    &asked,
                )
                .await
            }
        };
        let (project, service) = match made {
            Ok(made) => made,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
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

    /// Restore this database into a copy beside it.
    ///
    /// **One database, not the node.** What arrives is a new service in
    /// this project holding one volume out of one backup, with the
    /// original still running and still holding its own data. Asked for
    /// by Jorge: the window card said "any moment between these two" and
    /// there was nothing to press.
    ///
    /// The deployment is queued here, because the rows alone are a
    /// database that never starts: `restore_into` writes what to unpack
    /// and where to stop, and the unpacking happens at the first
    /// deployment. An operator who pressed this asked for a database, not
    /// for a service they then have to notice and deploy.
    #[post("/projects/:project/services/:service/restore")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn restore(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, back)) =
            crate::features::services::service_rest_controller::locate_for(
                &self.state,
                &self.auth,
                &path,
            )
            .await?
        else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let restored = match crate::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::databases::node_user_restore_database::run(
                    &self.state.database,
                    &self.state.config,
                    &account,
                    &service,
                    field(&form, "at"),
                    field(&form, "into"),
                )
                .await
            }
            false => {
                crate::features::databases::tenant_user_restore_database::run(
                    &self.state.database,
                    &self.state.config,
                    &account,
                    &service,
                    field(&form, "at"),
                    field(&form, "into"),
                )
                .await
            }
        };
        let made = match restored {
            Ok(made) => made,
            Err(reason) => return Ok(back_with_error(&back, &reason)),
        };

        let command = crate::deploy::jobs::DeployService {
            service_id: made.service_id.clone(),
            release_id: None,
        };
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            tracing::error!(service = %made.service_id, %error, "could not queue a deployment");
        }
        tracing::info!(
            from = %service.slug,
            into = %made.slug,
            backup = %made.from,
            target = made.target.as_deref().unwrap_or("the end of the log"),
            "restoring a database into a copy"
        );
        // To the copy, not back to the original: what somebody wants to
        // look at next is the thing that is coming up.
        Ok(see_other(&format!(
            "/projects/{}/services/{}",
            project.slug, made.slug
        )))
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
    project: &crate::entities::project::Project,
    service: &crate::entities::service::Service,
) -> String {
    // The tab, not the bare path: `/settings` is a redirect to the first
    // tab now, and a save that lands on a redirect is a save whose
    // fragment is dropped on the way — which is the same "a save that
    // hides what it did" this function exists to prevent.
    crate::features::services::service_settings_page::Tab::Database
        .path(&project.slug, &service.slug)
}

#[cfg(test)]
mod tests {
    use crate::console::tests::Console;
    use crate::repositories::databases;
    use crate::repositories::services;
    use hypertext::Renderable;
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
        let project = crate::repositories::projects::create(&database, "db-test")
            .await
            .expect("project");
        let (service, row) = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&database)
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
        // them and what `crate::features::databases::database_cards_component::Strings::of` reads the four choices out of.
        assert_eq!(
            names.first().map(String::as_str),
            Some("orders.db-test.node.example")
        );

        let replicas = crate::repositories::replicas::of_service(&database, &service.id)
            .await
            .expect("replicas");
        let card = crate::features::databases::database_cards_component::database_card(
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
        let project = crate::repositories::projects::create(&database, "db-test")
            .await
            .expect("project");
        let (service, row) = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&database)
        .await
        .expect("created");
        let names = crate::deploy::certificate_names(
            &database,
            &crate::config::Config::default(),
            &project,
            &service,
        )
        .await;
        let replicas = crate::repositories::replicas::of_service(&database, &service.id)
            .await
            .expect("replicas");
        let address = Some("10.42.2.200".to_string());

        let public = crate::features::databases::database_cards_component::database_card(
            &row,
            &replicas,
            address.clone(),
            None,
            &names,
            false,
            None,
        )
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
        let bare = crate::features::databases::database_cards_component::database_card(
            &row,
            &replicas,
            address,
            None,
            &short_only,
            true,
            None,
        )
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
        let project = crate::repositories::projects::create(&database, "db-test")
            .await
            .expect("project");
        let (service, _) = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&database)
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

        let port = crate::repositories::ports::of_service(&database, &service.id)
            .await
            .expect("ports")
            .pop()
            .expect("a database is created with one");
        crate::repositories::ports::set_hostname(&database, &port.id, Some("db.example.com"))
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
        crate::repositories::projects::create(&console.database, "shared")
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
        assert_eq!(service.kind, crate::entities::service::Kind::Postgres);
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
        let (service, _) = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project_id,
            name: "orders",
            version: "17",
            memory_limit: 64 * 1024 * 1024,
        }
        .run(&console.database)
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

    async fn my_api(console: &Console, cookie: &str) {
        console
            .harness
            .post("/projects")
            .header("cookie", cookie)
            .form(&[("name", "My API")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
    }

    /// The window said "any moment between these two" and there was
    /// nowhere to go with it.
    ///
    /// Jorge found it on the node: the card promised a restore and the
    /// console had no form for one — `restore` was a command, so
    /// recovering meant an SSH session in the middle of the thing you
    /// were recovering from. What arrives is **one database**, in a copy
    /// beside the original.
    #[tokio::test]
    async fn a_database_is_restored_into_a_copy_beside_it() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        my_api(&console, &cookie).await;
        let project = crate::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&console.database)
        .await
        .expect("a database");

        // No backup on the disk: no form, because it could only refuse.
        let bare = console
            .harness
            .get("/projects/my-api/services/orders")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            !bare.contains("/services/orders/restore"),
            "nothing to restore from, so nothing to press: {bare}"
        );

        // A backup that holds it, as `backup` would have written one.
        let taken_at = crate::clock::now_ms() - 3_600_000;
        let into = console
            .data_dir
            .join("backups")
            .join("nodes")
            .join("nd-test")
            .join(taken_at.to_string());
        std::fs::create_dir_all(into.join("volumes").join("my-api.orders"))
            .expect("a backup directory");
        std::fs::write(
            into.join("volumes")
                .join("my-api.orders")
                .join("base.tar.gz"),
            "not really a tarball",
        )
        .expect("the copy");
        std::fs::write(
            into.join("manifest.json"),
            serde_json::json!({
                "format": 1,
                "taken_by": "wabot-deploy 0.0.0-test",
                "taken_at": taken_at,
                "node_id": "nd-test",
                "node_name": "test",
                "volumes": [{
                    "container": "my-api.orders",
                    "bytes": 20,
                    "how": "Consistent",
                }],
                "images": [],
            })
            .to_string(),
        )
        .expect("the manifest");

        let offered = console
            .harness
            .get("/projects/my-api/services/orders")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            offered.contains("/projects/my-api/services/orders/restore"),
            "now there is something to restore from: {offered}"
        );

        let response = console
            .harness
            .post("/projects/my-api/services/orders/restore")
            .header("cookie", &cookie)
            .form(&[("at", ""), ("into", "orders-friday")])
            .send()
            .await;
        // To the copy, which is the thing somebody wants to look at next.
        assert_eq!(
            response.header("location"),
            Some("/projects/my-api/services/orders-friday")
        );

        let copy = services::all(&console.database, None)
            .await
            .expect("query")
            .into_iter()
            .find(|service| service.slug == "orders-friday")
            .expect("the copy exists");
        let row = crate::repositories::databases::of_service(&console.database, &copy.id)
            .await
            .expect("query")
            .expect("an engine row");
        assert_eq!(
            row.restored_from.as_deref(),
            Some(
                into.join("volumes")
                    .join("my-api.orders")
                    .to_string_lossy()
                    .as_ref()
            ),
            "it knows which backup to unpack"
        );
        // Empty meant "as far as the log goes", which is no target at all
        // — not the string "", which would be refused as a moment.
        assert_eq!(row.recovery_target, None);

        // And the original is untouched: same rows, still there.
        let original = services::all(&console.database, None)
            .await
            .expect("query")
            .into_iter()
            .find(|service| service.slug == "orders")
            .expect("the original is still a service");
        assert!(
            crate::repositories::databases::of_service(&console.database, &original.id)
                .await
                .expect("query")
                .is_some()
        );
    }

    /// The backup is not on this node any more — it is at the
    /// destination.
    ///
    /// Jorge asked what happens then, and the answer used to be nothing:
    /// the search read this node's own root, and a schedule with a
    /// destination removes the local copy the moment the transfer
    /// succeeds. So a node that had been backing itself up all week would
    /// be told by the machine that took them that it has no backups.
    ///
    /// Asked here with a **local** destination in another directory,
    /// which is the same code path a bucket takes: `index` reads a root
    /// this node did not write, `holds` answers from the manifest rather
    /// than the disk — that is what makes it answerable of a bucket
    /// without downloading anything — and the row records where the
    /// volume comes from so the deployment can fetch it.
    #[tokio::test]
    async fn a_database_is_restored_from_the_destination_when_nothing_is_here() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        my_api(&console, &cookie).await;
        let project = crate::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&console.database)
        .await
        .expect("a database");

        // A root that is not this node's own — `data_dir/backups` is
        // where its own would be, and there is nothing there.
        let away = console.data_dir.join("somewhere-else");
        let taken_at = crate::clock::now_ms() - 7_200_000;
        let backup = away
            .join("nodes")
            .join("nd-test")
            .join(taken_at.to_string());
        std::fs::create_dir_all(backup.join("volumes").join("my-api.orders"))
            .expect("a backup directory");
        std::fs::write(
            backup
                .join("volumes")
                .join("my-api.orders")
                .join("base.tar.gz"),
            "not really a tarball",
        )
        .expect("the copy");
        std::fs::write(
            backup.join("manifest.json"),
            serde_json::json!({
                "format": 1,
                "taken_by": "wabot-deploy 0.0.0-test",
                "taken_at": taken_at,
                "node_id": "nd-test",
                "node_name": "test",
                "volumes": [{
                    "container": "my-api.orders",
                    "bytes": 20,
                    "how": "Consistent",
                }],
                "images": [],
            })
            .to_string(),
        )
        .expect("the manifest");
        crate::node::backups::store(
            &console.database,
            &crate::node::backups::Plan {
                destination: Some(away.display().to_string()),
                ..crate::node::backups::Plan::default()
            },
        )
        .await
        .expect("the plan");

        // The card is offered although nothing is here: asking the
        // destination on every page render would make this page depend on
        // somebody's network, so the search happens on submit.
        let body = console
            .harness
            .get("/projects/my-api/services/orders")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("/projects/my-api/services/orders/restore"),
            "the form is offered: {body}"
        );

        let response = console
            .harness
            .post("/projects/my-api/services/orders/restore")
            .header("cookie", &cookie)
            .form(&[("at", ""), ("into", "orders-from-away")])
            .send()
            .await;
        assert_eq!(
            response.header("location"),
            Some("/projects/my-api/services/orders-from-away")
        );

        let copy = services::all(&console.database, None)
            .await
            .expect("query")
            .into_iter()
            .find(|service| service.slug == "orders-from-away")
            .expect("the copy exists");
        let row = crate::repositories::databases::of_service(&console.database, &copy.id)
            .await
            .expect("query")
            .expect("an engine row");
        assert_eq!(
            row.restored_from.as_deref(),
            Some(
                backup
                    .join("volumes")
                    .join("my-api.orders")
                    .to_string_lossy()
                    .as_ref()
            ),
            "the row says where the volume comes from, wherever that is"
        );
    }

    /// A moment nothing can reach is refused, with the reason on the page
    /// somebody typed it into.
    ///
    /// Replaying only goes forwards, so a backup taken after the moment
    /// asked for already holds whatever is being undone. The refusal says
    /// which directory it looked in — in the middle of a recovery, "you
    /// have nothing" had better name where it looked.
    #[tokio::test]
    async fn a_moment_no_backup_can_reach_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        my_api(&console, &cookie).await;
        let project = crate::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&console.database)
        .await
        .expect("a database");

        let refused = console
            .harness
            .post("/projects/my-api/services/orders/restore")
            .header("cookie", &cookie)
            .form(&[("at", "2026-08-16 14:32"), ("into", "")])
            .send()
            .await;
        let location = refused.header("location").unwrap_or_default().to_string();
        assert!(location.contains("error="), "{location}");
        assert!(
            location.contains("backups"),
            "and where it looked: {location}"
        );

        // Nothing was created for a restore that could not start.
        assert!(
            services::all(&console.database, None)
                .await
                .expect("query")
                .iter()
                .all(|service| service.slug != "orders-restored"),
            "a refused restore leaves no half-made database"
        );

        // And a moment that is not one says so rather than guessing.
        let nonsense = console
            .harness
            .post("/projects/my-api/services/orders/restore")
            .header("cookie", &cookie)
            .form(&[("at", "last tuesday"), ("into", "")])
            .send()
            .await;
        assert!(
            nonsense
                .header("location")
                .unwrap_or_default()
                .contains("not+a+moment"),
            "{:?}",
            nonsense.header("location")
        );
    }
}
