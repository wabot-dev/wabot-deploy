//! What a service's forms post to, and the log its page streams.

use std::sync::Arc;

use crate::features::services::service_settings_page::Tab;
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::infra::repositories::access;
use crate::infra::repositories::services;

use crate::shell::auth::{
    back_with_error, field, local_path, read_form, see_other, signed_in, SessionMiddleware,
};
use crate::shell::state::ConsoleState;

/// The value the placement form sends for "let the node decide".
///
/// A word rather than an empty field, because an empty field already
/// means *here* and has since the per-slot selectors were written. Two
/// intentions that differ have to be two values, or changing one of them
/// silently changes what every existing form does.
pub(crate) const ANYWHERE: &str = "anywhere";

/// One instalment of a log, as the stream sends it.
#[derive(Debug, serde::Serialize)]
struct LogChunk {
    text: String,
    /// The file got **shorter** than where the reader was, so what is on
    /// screen may not be the beginning of what is there now and the
    /// reader is told rather than having new output appended to old.
    ///
    /// Not "a deployment": a deployment appends. This is a rotation at a
    /// container start, or the sweep trimming a live file that never
    /// restarts — the two things that make a log lose its front.
    restarted: bool,
}

#[injectable]
pub struct ServiceRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl ServiceRestController {
    /// The same log, following.
    ///
    /// Sends only what arrived since the offset it was given, so a page
    /// left open all afternoon costs one read of the tail of a file every
    /// second — not the whole file, and not the part already on screen.
    ///
    /// A **restart** is said rather than smoothed over: a deployment
    /// empties the file, and what is on the reader's screen at that
    /// moment belongs to a container that no longer exists.
    #[get("/projects/:project/services/:service/logs/live")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn logs_live(&self, request: Request) -> RestResult<Response> {
        let empty = |status: StatusCode| {
            Ok(Response::builder()
                .status(status)
                .body(Body::empty())
                .expect("a constant response is well-formed"))
        };
        // A status, not a redirect: an EventSource cannot follow one
        // usefully, and a stream is not a page.
        let Some(account) = signed_in(&self.auth) else {
            return empty(StatusCode::UNAUTHORIZED);
        };

        let path = request.uri().path().to_string();
        let segments = crate::shell::auth::segments(&path);
        let (Some(project_slug), Some(service_slug)) = (segments.get(1), segments.get(3)) else {
            return empty(StatusCode::NOT_FOUND);
        };
        // The same check the page makes. A stream that skipped it would
        // be a way to read the output of a project somebody has no
        // access to, which is a worse hole than the page it belongs to.
        let Some((project, _)) = access::find_project(&self.state.database, &account, project_slug)
            .await
            .ok()
            .flatten()
        else {
            return empty(StatusCode::NOT_FOUND);
        };
        let Some(service) = services::in_project(&self.state.database, &project.id, service_slug)
            .await
            .ok()
            .flatten()
        else {
            return empty(StatusCode::NOT_FOUND);
        };

        let query: std::collections::HashMap<String, String> =
            form_urlencoded::parse(request.uri().query().unwrap_or_default().as_bytes())
                .into_owned()
                .collect();
        let slot: u32 = query
            .get("slot")
            .and_then(|slot| slot.parse().ok())
            .unwrap_or(1);
        let from: u64 = query
            .get("from")
            .and_then(|from| from.parse().ok())
            .unwrap_or(0);

        // Only a copy that runs here, and only one that exists. Reading
        // a log by slot number alone would let a query string name a
        // container in another project.
        let holds =
            crate::infra::repositories::replicas::of_service(&self.state.database, &service.id)
                .await
                .unwrap_or_default()
                .into_iter()
                .any(|replica| replica.is_here() && !replica.evicted() && replica.slot == slot);
        if !holds {
            return empty(StatusCode::NOT_FOUND);
        }

        let container =
            crate::domain::entities::replica::container_id_for(&project.slug, &service.slug, slot);
        let data_dir = self.state.config.node.data_dir.clone();
        let stream = async_stream::stream! {
            let mut at = from;
            loop {
                if let Some(chunk) = crate::infra::deploy::logs::read_from(&data_dir, &container, at) {
                    at = chunk.next;
                    // Nothing to say is not an event. A heartbeat is not
                    // needed either: the browser reconnects on its own,
                    // and an empty event per second would be a page that
                    // never idles.
                    if !chunk.text.is_empty() || chunk.restarted {
                        let payload = serde_json::to_string(&LogChunk {
                            text: chunk.text,
                            restarted: chunk.restarted,
                        })
                        .unwrap_or_else(|_| "{}".into());
                        yield Ok::<_, std::convert::Infallible>(
                            wabot::rest::axum::body::Bytes::from(format!("data: {payload}\n\n")),
                        );
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            }
        };

        Ok(Response::builder()
            .header(header::CONTENT_TYPE, "text/event-stream")
            // Every hop in between has to be told, or a proxy holds the
            // stream until it has "enough" and the page never updates.
            .header(header::CACHE_CONTROL, "no-cache")
            .header("x-accel-buffering", "no")
            .body(Body::from_stream(stream))
            .expect("a constant response is well-formed"))
    }

    #[post("/projects/:project/services")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };

        // The slug out of the path. Taken from the URI rather than a
        // typed request because `#[raw]` hands over the whole thing —
        // the trade for being able to answer with a 303.
        let path = request.uri().path().to_string();
        let Some(slug) = crate::shell::auth::project_slug(&path) else {
            return Ok(see_other("/?error=no+such+project"));
        };
        let Some((project, _)) = access::find_project(&self.state.database, &account, slug).await?
        else {
            return Ok(see_other("/?error=no+such+project"));
        };
        let form_url = format!("/projects/{}/services/new", project.slug);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let env = match parse_env(field(&form, "env")) {
            Ok(env) => env,
            Err(message) => return Ok(back_with_error(&form_url, &message)),
        };

        // Built here and validated here: the use case is handed a
        // request whose *shape* is already right, and asks only the
        // questions a form cannot answer.
        let asked = crate::features::services::create_service_request::CreateServiceRequest {
            project: slug.to_string(),
            name: field(&form, "name").to_string(),
            image: field(&form, "image").to_string(),
            env: field(&form, "env").to_string(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&form_url, &reason.to_string()));
        }

        let made = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_create_service::run(
                    &self.state.database,
                    &account,
                    &asked,
                    &env,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_create_service::run(
                    &self.state.database,
                    &account,
                    &asked,
                    &env,
                )
                .await
            }
        };
        match made {
            Ok(made) => {
                // Deployed on creation. The alternative — create, then
                // press Deploy — makes the common case two steps and
                // leaves a row that describes nothing.
                //
                // Straight to the service's own page: declaring ports
                // is the next thing anybody does, and it is where the
                // reason lives if this deployment failed.
                self.enqueue(&made.service_id, None).await;
                Ok(see_other(&format!(
                    "/projects/{}/services/{}",
                    made.project_slug, made.service_slug
                )))
            }
            Err(reason) => Ok(back_with_error(&form_url, &reason)),
        }
    }

    /// Declare a port on a service.
    ///
    /// A hostname is checked against DNS *before* it is stored. The
    /// alternative — store it, then find out at certificate time — is a
    /// service that looks configured and answers nothing, with the
    /// reason in a log the operator never sees.
    #[post("/projects/:project/services/:service/ports")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_port(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings/network",
            project.slug, service.slug
        );

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let container_port = match parse_port(field(&form, "container_port")) {
            Ok(Some(port)) => port,
            Ok(None) => return Ok(back_with_error(&here, "a container port is required")),
            Err(message) => return Ok(back_with_error(&here, message)),
        };
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let asked = crate::features::services::add_service_port_request::AddServicePortRequest {
            container_port,
            publish: checked(&form, "publish"),
            hostname: field(&form, "hostname").to_string(),
            https: checked(&form, "https"),
        };
        let opened = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_add_service_port::run(
                    &self.state.database,
                    &account,
                    &service,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_add_service_port::run(
                    &self.state.database,
                    &account,
                    &service,
                    &asked,
                )
                .await
            }
        };
        match opened {
            Ok(port) => {
                // **And tell whoever answers for the name.** The row
                // `ports::create` writes can now name *another* node —
                // the one the hostname actually resolves to — and nothing
                // here used to tell it. Found on the node: a service on
                // the private machine with its name pointing at the
                // public one, the edge row written a millisecond after
                // the port, and the public node never asked to serve it.
                // Every deployment does this too; this is for the
                // operator who just pressed the button.
                self.state.deployer.tell_edges(&service).await;

                // **The source before the first order.** `ports::create`
                // writes the route, and the certificate loop acts on it
                // within seconds — so a policy stored after this would
                // arrive too late to stop an order the operator did not
                // want. A failed order counts against an account that
                // locks at five an hour.
                //
                // Refused rather than defaulted when the paths are wrong:
                // `source_from` reads the files now, while there is
                // somebody to tell. The port is already made by then, so
                // the message says so — losing the port would be worse
                // than a certificate that needs a second visit.
                if let Some(hostname) = port.hostname.as_deref() {
                    match crate::shell::auth::source_from(&form, hostname) {
                        Ok(renew_with) => {
                            if let Err(error) = crate::infra::edge::policy::store_source(
                                &self.state.database,
                                hostname,
                                &renew_with,
                            )
                            .await
                            {
                                tracing::error!(%error, "could not store the certificate source");
                            }
                        }
                        Err(reason) => {
                            self.enqueue(&service.id, None).await;
                            return Ok(back_with_error(
                                &here,
                                &format!(
                                    "the port was added, and the certificate was left at the \
                                     default: {reason}"
                                ),
                            ));
                        }
                    }
                }

                // Redeployed, because a port is part of how the
                // container is built: published ports are iptables
                // rules made when it joins the network, not something
                // that can be added to a running one.
                self.enqueue(&service.id, None).await;

                let confirmed = port
                    .hostname
                    .map(|hostname| format!("{hostname} resolves here, and is now routed"));
                // The fragment last, after any query: it is where the
                // card is, so the page comes back where it was rather
                // than at the top. See `back_with_error`, which does
                // not carry one — a refusal belongs beside the note
                // that explains it, and that note is at the top.
                Ok(see_other(&match confirmed {
                    Some(message) => format!(
                        "{here}?{}#ports",
                        form_urlencoded::Serializer::new(String::new())
                            .append_pair("checked", &message)
                            .finish()
                    ),
                    None => format!("{here}#ports"),
                }))
            }
            Err(error) => Ok(back_with_error(&here, &error.to_string())),
        }
    }

    /// Remove a port.
    #[post("/projects/:project/services/:service/ports/:port/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete_port(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let segments = crate::shell::auth::segments(&path);
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings/network",
            project.slug, service.slug
        );

        let ["projects", _, "services", _, "ports", port_id, "delete"] = segments.as_slice() else {
            return Ok(see_other(&here));
        };

        // Only this service's ports, or an id from another page would
        // delete somebody else's route.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let removed = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_delete_service_port::run(
                    &self.state.database,
                    &account,
                    &service,
                    port_id,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_delete_service_port::run(
                    &self.state.database,
                    &account,
                    &service,
                    port_id,
                )
                .await
            }
        };
        match removed {
            Ok(()) => {
                // Now, not when the redeploy gets there: the other nodes
                // that answered for the port's name are told to stop by
                // the route sync, and a deployment that fails first would
                // leave them answering.
                self.state.deployer.sync_routes().await;
                self.enqueue(&service.id, None).await;
                Ok(see_other(&format!("{here}#ports")))
            }
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/services/:service/volumes")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn add_volume(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let here = format!(
            "/projects/{}/services/{}/settings/storage",
            project.slug, service.slug
        );
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let asked =
            crate::features::services::add_service_volume_request::AddServiceVolumeRequest {
                path: field(&form, "path").to_string(),
                name: field(&form, "name").to_string(),
            };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }
        let added = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_add_service_volume::run(
                    &self.state.database,
                    &account,
                    &service,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_add_service_volume::run(
                    &self.state.database,
                    &account,
                    &service,
                    &asked,
                )
                .await
            }
        };
        match added {
            Ok(_) => {
                self.enqueue(&service.id, None).await;
                Ok(see_other(&format!("{here}#volumes")))
            }
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/projects/:project/services/:service/volumes/:volume/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete_volume(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let segments = crate::shell::auth::segments(&path);
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let here = format!(
            "/projects/{}/services/{}/settings/storage",
            project.slug, service.slug
        );
        let ["projects", _, "services", _, "volumes", volume_id, "delete"] = segments.as_slice()
        else {
            return Ok(see_other(&here));
        };
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let removed = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_delete_service_volume::run(
                    &self.state.database,
                    &account,
                    &service,
                    volume_id,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_delete_service_volume::run(
                    &self.state.database,
                    &account,
                    &service,
                    volume_id,
                )
                .await
            }
        };
        match removed {
            Ok(()) => {
                self.enqueue(&service.id, None).await;
                Ok(see_other(&format!("{here}#volumes")))
            }
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    /// Deploy one release — the newest, or an older one, which is
    /// what a rollback is.
    #[post("/projects/:project/services/:service/releases/:release/deploy")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn deploy_release(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let segments = crate::shell::auth::segments(&path);
        let Some((project, service, back)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let here = format!("/projects/{}/services/{}", project.slug, service.slug);
        let _ = back;

        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let Some(id) = segments.get(5) else {
            return Ok(see_other(&here));
        };

        let chosen = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_deploy_release::run(
                    &self.state.database,
                    &account,
                    &service,
                    id,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_deploy_release::run(
                    &self.state.database,
                    &account,
                    &service,
                    id,
                )
                .await
            }
        };
        match chosen {
            Ok(release) => {
                self.enqueue(&service.id, Some(&release.id)).await;
                Ok(see_other(&here))
            }
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    /// Point the service at a different image, and redeploy with it.
    ///
    /// **There was no way to do this**, which is the fault Jorge found
    /// while being told by this very page to point a service at a
    /// reference: the field existed only on the creation form, so a
    /// service was pinned for life to whatever it was made with.
    ///
    /// Empty is allowed and means "wait for a push to your own
    /// repository" — the deploy path says so on the row rather than
    /// attempting a pull with no reference.
    #[post("/projects/:project/services/:service/image")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_image(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let here = format!(
            "/projects/{}/services/{}/settings/image",
            project.slug, service.slug
        );

        // The check that counts. This page does not show the form for a
        // managed database — the node writes that row from the engine's —
        // and a rule the browser enforces is a courtesy.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let outcome = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_set_service_image::run(
                    &self.state.database,
                    &account,
                    &service,
                    field(&form, "image"),
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_set_service_image::run(
                    &self.state.database,
                    &account,
                    &service,
                    field(&form, "image"),
                )
                .await
            }
        };
        if let Err(reason) = outcome {
            return Ok(back_with_error(&here, &reason));
        }

        let command = crate::infra::deploy::jobs::DeployService {
            service_id: service.id.clone(),
            release_id: None,
        };
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            tracing::error!(%error, "could not queue the deployment");
        }
        Ok(see_other(&format!("{here}#image")))
    }

    /// What tag to watch, and whether a push goes out on its own.
    #[post("/projects/:project/services/:service/tracking")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn tracking(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings/image",
            project.slug, service.slug
        );

        // The check that counts. The settings page does not show this
        // form for a managed database, and a rule the browser enforces
        // is a courtesy — what the node writes from the engine's row
        // must not be settable from a request somebody constructed.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let outcome = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_set_service_tracking::run(
                    &self.state.database,
                    &account,
                    &service,
                    field(&form, "track_tag"),
                    checked(&form, "auto_deploy"),
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_set_service_tracking::run(
                    &self.state.database,
                    &account,
                    &service,
                    field(&form, "track_tag"),
                    checked(&form, "auto_deploy"),
                )
                .await
            }
        };
        if let Err(reason) = outcome {
            return Ok(back_with_error(&here, &reason));
        }
        Ok(see_other(&format!("{here}#releases")))
    }

    /// Whether this service's output is timestamped as it is written.
    ///
    /// **No deployment is queued.** The other settings forms redeploy,
    /// because a memory ceiling or an environment variable is meaningless
    /// until the container is recreated with it. This one is the same in
    /// principle and different in kind: it puts a process between a
    /// running container and its log, so restarting somebody's service
    /// the instant they tick a box would be the switch doing more than it
    /// said. The form says it takes effect at the next deployment, and
    /// that is exactly what happens.
    #[post("/projects/:project/services/:service/log-timestamps")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn log_timestamps(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let here = format!(
            "/projects/{}/services/{}/settings/logs",
            project.slug, service.slug
        );

        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let outcome = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_set_service_logs::run(
                    &self.state.database,
                    &account,
                    &service,
                    field(&form, "log_budget"),
                    checked(&form, "log_timestamps"),
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_set_service_logs::run(
                    &self.state.database,
                    &account,
                    &service,
                    field(&form, "log_budget"),
                    checked(&form, "log_timestamps"),
                )
                .await
            }
        };
        if let Err(reason) = outcome {
            return Ok(back_with_error(&here, &reason));
        }
        Ok(see_other(&format!("{here}#logs")))
    }

    /// Change the environment, and redeploy with it.
    /// How much memory this service's containers may have.
    ///
    /// Redeploys, because a cgroup limit is written when a container is
    /// created and there is nothing to change on a running one. Saying
    /// so and doing nothing would be a page that shows a number the
    /// container does not have.
    #[post("/projects/:project/services/:service/memory")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_memory(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/"));
        };
        let here = format!(
            "/projects/{}/services/{}/settings/resources",
            project.slug, service.slug
        );

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        // The form asks the two kinds differently, so the POST reads them
        // differently — a database's ceiling is a rung because it is also
        // an engine configuration, and a plain container's is whatever the
        // node can keep. The check that counts is here either way: the
        // select is a courtesy the browser offers and a POST can be made
        // by hand.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let asked =
            crate::features::services::set_service_resources_request::SetServiceResourcesRequest {
                memory: field(&form, "memory").to_string(),
                cpu: field(&form, "cpu").to_string(),
                disk: field(&form, "disk").to_string(),
            };
        let changed = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_set_service_resources::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &service,
                    &asked,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_set_service_resources::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &service,
                    &asked,
                )
                .await
            }
        };
        let fresh = match changed {
            Ok(fresh) => fresh,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        self.state
            .deployer
            .tell_holders_including(&project, &fresh, &Default::default())
            .await;

        let command = crate::infra::deploy::jobs::DeployService {
            service_id: service.id.clone(),
            release_id: None,
        };
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            tracing::error!(%error, "could not queue the deployment");
        }
        Ok(see_other(&format!("{here}#memory")))
    }

    #[post("/projects/:project/services/:service/env")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_env(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings/environment",
            project.slug, service.slug
        );

        // The check that counts. The settings page does not show this
        // form for a managed database, and a rule the browser enforces
        // is a courtesy — what the node writes from the engine's row
        // must not be settable from a request somebody constructed.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let env = match parse_env(field(&form, "env")) {
            Ok(env) => env,
            Err(message) => return Ok(back_with_error(&here, &message)),
        };
        let applied = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_set_service_env::run(
                    &self.state.database,
                    &account,
                    &service,
                    env,
                    "edit",
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_set_service_env::run(
                    &self.state.database,
                    &account,
                    &service,
                    env,
                    "edit",
                )
                .await
            }
        };
        let applied = match applied {
            Ok(applied) => applied,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        self.enqueue(&applied.service_id, applied.release_id.as_deref())
            .await;
        Ok(see_other(&format!("{here}#environment")))
    }

    /// Put an earlier set of values back.
    ///
    /// Independent of releases on purpose: the usual case is "this
    /// build is bad, run the previous one, keep the settings I fixed
    /// since" — and the other way round just as often.
    #[post("/projects/:project/services/:service/env/:revision/revert")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revert_env(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let segments = crate::shell::auth::segments(&path);
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        // Back to the page the form is on, not the one it used to
        // share with the service's state.
        let here = format!(
            "/projects/{}/services/{}/settings/environment",
            project.slug, service.slug
        );
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let Some(id) = segments.get(5) else {
            return Ok(see_other(&here));
        };

        let chosen = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_revert_service_env::run(
                    &self.state.database,
                    &account,
                    &service,
                    id,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_revert_service_env::run(
                    &self.state.database,
                    &account,
                    &service,
                    id,
                )
                .await
            }
        };
        let env = match chosen {
            Ok(env) => env,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        let applied = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_set_service_env::run(
                    &self.state.database,
                    &account,
                    &service,
                    env,
                    "revert",
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_set_service_env::run(
                    &self.state.database,
                    &account,
                    &service,
                    env,
                    "revert",
                )
                .await
            }
        };
        let applied = match applied {
            Ok(applied) => applied,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };
        self.enqueue(&applied.service_id, applied.release_id.as_deref())
            .await;
        Ok(see_other(&format!("{here}#environment")))
    }

    /// Start (or restart) a service's container.
    #[post("/projects/:project/services/:service/deploy")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn deploy(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, back)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let back = returning_to(request, &back).await;

        // The failure lands on the row, not in this redirect: the job
        // outlives the answer, and a reason carried in a query string
        // is one the next click loses anyway.
        let _ = project;
        self.enqueue(&service.id, None).await;
        Ok(see_other(&back))
    }

    /// Stop a service and take it off its project's network.
    #[post("/projects/:project/services/:service/stop")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn stop(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, back)) = self.locate(&path).await? else {
            return Ok(see_other("/?error=no+such+service"));
        };
        let back = returning_to(request, &back).await;

        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let stopped = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_stop_service::run(
                    &self.state.deployer,
                    &account,
                    &project,
                    &service,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_stop_service::run(
                    &self.state.deployer,
                    &account,
                    &project,
                    &service,
                )
                .await
            }
        };
        match stopped {
            Ok(()) => Ok(see_other(&back)),
            Err(reason) => Ok(back_with_error(&back, &reason)),
        }
    }

    /// Remove a service, and the container behind it.
    #[post("/projects/:project/services/:service/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let Some((project, service, back)) = self.locate(request.uri().path()).await? else {
            // Already gone is the outcome they asked for.
            return Ok(see_other("/"));
        };
        let here = Tab::Danger.path(&project.slug, &service.slug);

        // **The check that counts.** The form asks for the name to be
        // typed and marks the field `required`, and both of those are
        // courtesies the browser extends — this console has to work with
        // scripting off and a request can be constructed by hand, so the
        // comparison happens here or it does not happen.
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let removed = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_delete_service::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &service,
                    field(&form, "confirm"),
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_delete_service::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &service,
                    field(&form, "confirm"),
                )
                .await
            }
        };
        match removed {
            Ok(()) => Ok(see_other(&back)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }
    /// Throw a service somebody else placed here off this machine.
    ///
    /// The one thing this node's operator can always do to something
    /// they did not create — the machine is theirs even when the orders
    /// are not, which is the same rule the grant follows in the other
    /// direction.
    ///
    /// **The guard is the inverse of `locate`'s.** That one refuses a
    /// foreign service because every caller of it changes something
    /// this node does not decide; this one refuses a service of *our
    /// own*, because deleting one of those is what the delete button is
    /// for and it has different consequences.
    ///
    /// The rows stay, marked evicted. They are what the next report
    /// carries, and that report is the only thing that makes the node
    /// which placed it stop asking — deleting them here would leave the
    /// authority re-sending an errand for a container this machine has
    /// just thrown out.
    #[post("/projects/:project/services/:service/evict")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn evict(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        // Not a project-level decision: what is being thrown out was
        // put here by another node, and answering to it is the node's
        // business rather than one project's.
        if !account.is_admin() {
            return Ok(see_other("/"));
        }

        let path = request.uri().path().to_string();
        let Some((project_slug, service_slug)) = service_path(&path) else {
            return Ok(see_other("/"));
        };
        let Some(project) = crate::infra::repositories::projects::all(&self.state.database)
            .await?
            .into_iter()
            .find(|project| project.slug == project_slug)
        else {
            return Ok(see_other("/"));
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, service_slug).await?
        else {
            return Ok(see_other("/"));
        };
        let here = format!("/projects/{}/services/{}", project.slug, service.slug);

        // The inverse guard. A service of this node's own is deleted,
        // not evicted, and the two are not the same button.
        let evicted = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_evict_service::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &service,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_evict_service::run(
                    &self.state.database,
                    &self.state.deployer,
                    &account,
                    &project,
                    &service,
                )
                .await
            }
        };
        match evicted {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    /// How many copies of this service run, and where each one goes.
    ///
    /// One form for both, because they are one decision: a replica is a
    /// slot with a node in it, and asking for three of them while
    /// naming where two go would be a page that could be submitted
    /// half-answered.
    ///
    /// The errand for a replica placed elsewhere is queued here rather
    /// than by a background pass. The operator pressed a button, and
    /// the thing they asked for should exist before the response comes
    /// back — a queue that noticed later would leave the page showing
    /// nothing happened.
    #[post("/projects/:project/services/:service/placement")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_placement(&self, request: Request) -> RestResult<Response> {
        // Needed for the credential below: a push token is minted *by*
        // somebody, and the column that says who is a foreign key to an
        // account. A placeholder there fails the insert, which
        // `dispatch` then reports as a warning nobody reads — so the
        // errand simply never appeared.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let path = request.uri().path().to_string();
        let Some((project, service, back)) = self.locate(&path).await? else {
            return Ok(see_other("/"));
        };
        // Back to the page the form is on. Where the copies go is a
        // decision, so it lives in settings — and landing somebody on
        // the detail page after they saved one is the console deciding
        // they had finished.
        let here = format!(
            "/projects/{}/services/{}/settings/replicas",
            project.slug, service.slug
        );
        let _ = back;

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let on = crate::features::services::set_service_placement_command::On {
            database: &self.state.database,
            deployer: &self.state.deployer,
            container: &self.state.container,
        };
        let command =
            crate::features::services::set_service_placement_command::SetServicePlacement {
                project: &project,
                service: &service,
                fields: &form,
                replicas: field(&form, "replicas"),
                on: field(&form, "new-on"),
                by: &account.id,
            };
        let placed = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_set_service_placement::run(
                    on, &account, command,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_set_service_placement::run(
                    on, &account, command,
                )
                .await
            }
        };
        if let Err(reason) = placed {
            return Ok(back_with_error(&here, &reason));
        }
        Ok(see_other(&format!("{here}#placement")))
    }

    /// Choose which public nodes answer for one of this service's
    /// names.
    ///
    /// A node dropped from the list is told too, with an empty upstream
    /// list — it keeps answering for the name until something says
    /// otherwise, and nothing else in the system would notice.
    #[post("/projects/:project/services/:service/edges")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_edges(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some((project, service, _)) = self.locate(&path).await? else {
            return Ok(see_other("/"));
        };
        // Settings, where the card is — see `placement` above.
        let here = format!(
            "/projects/{}/services/{}/settings/network",
            project.slug, service.slug
        );

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let hostname = field(&form, "hostname").to_string();
        if hostname.is_empty() {
            return Ok(back_with_error(&here, "that names no hostname"));
        }

        // A checkbox sends its value only when ticked, so the chosen
        // set is what arrived and the rest is what was cleared. Read
        // from the node table rather than from the form, so a field
        // naming something that is not a public node cannot put a row
        // in.
        let public: Vec<String> = crate::infra::network::edge_candidates(&self.state.database)
            .await?
            .into_iter()
            .map(|node| node.id)
            .collect();
        let chosen: Vec<String> = public
            .into_iter()
            .filter(|id| form.contains_key(&format!("edge-{id}")))
            .collect();

        // **And the name has to point at each of them.** Ticking a node
        // the name does not resolve to writes a row saying it answers,
        // sends it an errand it carries out, and produces a hostname this
        // console shows as served and nothing reaches. The same check the
        // port form makes, in the other place a name is bound to a node —
        // it was only ever asked in one of the two.
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let routed = match crate::domain::authz::node_user::belongs_here(&account) {
            true => {
                crate::features::services::node_user_set_service_edges::run(
                    &self.state.database,
                    &account,
                    &service,
                    &hostname,
                    &chosen,
                )
                .await
            }
            false => {
                crate::features::services::tenant_user_set_service_edges::run(
                    &self.state.database,
                    &account,
                    &service,
                    &hostname,
                    &chosen,
                )
                .await
            }
        };
        let dropped = match routed {
            Ok(dropped) => dropped,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        let placements =
            crate::infra::repositories::replicas::of_service(&self.state.database, &service.id)
                .await?;
        if let Err(reason) =
            crate::features::services::set_service_placement_command::dispatch_edges(
                &self.state.deployer,
                &service,
                &placements,
            )
            .await
        {
            return Ok(back_with_error(&here, &reason));
        }

        // The ones no longer chosen, told to stop: an empty list
        // releases the name there rather than leaving it answering for
        // something nobody asked it to serve.
        for node_id in dropped {
            crate::infra::network::errand::withdraw_name(&self.state.database, &node_id, &hostname)
                .await?;
        }

        Ok(see_other(&format!("{here}#edges")))
    }
}

impl ServiceRestController {
    /// Ask for a deployment and answer the browser.
    ///
    /// The reason a failure is not returned here: by the time one
    /// exists this request is long finished. `deploy` writes it to the
    /// service row, which is what the page reads — see
    /// [`crate::infra::deploy::jobs`].
    async fn enqueue(&self, service_id: &str, release_id: Option<&str>) {
        let command = crate::infra::deploy::jobs::DeployService {
            service_id: service_id.to_string(),
            release_id: release_id.map(str::to_string),
        };
        if let Err(error) = wabot::async_jobs::run_command(&self.state.container, &command).await {
            // Nothing else can report this: there is no job to carry
            // the reason, because making one is what failed.
            tracing::error!(%service_id, %error, "could not queue a deployment");
        }
    }

    /// The project and service this request is about, and where to send
    /// the browser afterwards.
    ///
    /// `None` when the session is missing or either slug names nothing —
    /// the caller turns that into a redirect, because at this point there
    /// is no page to render an error onto.
    ///
    /// Takes the path rather than the request: holding a `&Request`
    /// across an `await` would need `Body: Sync`, which it is not, and
    /// the handler quietly stops being one axum will accept.
    async fn locate(
        &self,
        path: &str,
    ) -> RestResult<
        Option<(
            crate::domain::entities::project::Project,
            crate::domain::entities::service::Service,
            String,
        )>,
    > {
        locate_for(&self.state, &self.auth, path).await
    }
}

/// The same locator, for a form that lives on this page and is handled
/// elsewhere — a database's certificate is one of them.
///
/// Free rather than a method so the database controller can use it without
/// inheriting the rest of this one: the permission question, the refusal
/// for a service that arrived on an errand and the page to go back to are
/// all the same, and answering them twice is how two forms on one page come
/// to disagree about who may submit them.
pub(crate) async fn locate_for(
    state: &Arc<ConsoleState>,
    auth: &Arc<Auth>,
    path: &str,
) -> RestResult<
    Option<(
        crate::domain::entities::project::Project,
        crate::domain::entities::service::Service,
        String,
    )>,
> {
    let Some(account) = signed_in(auth) else {
        return Ok(None);
    };
    let Some((project_slug, service_slug)) = service_path(path) else {
        return Ok(None);
    };
    let Some((project, allowed)) =
        access::find_project(&state.database, &account, project_slug).await?
    else {
        return Ok(None);
    };
    // Every caller of this mutates something. A viewer gets the same
    // answer as a stranger, which is the same answer as a service that
    // does not exist.
    if !allowed.may_deploy() {
        return Ok(None);
    }
    let found = services::in_project(&state.database, &project.id, service_slug).await?;

    // A service that arrived on an errand is administered from the node
    // that sent it. Every caller of this changes something, so a foreign
    // one answers the same way a stranger's does — the one thing this
    // node's operator can do to it is throw it out, and that has its own
    // path through the danger zone rather than going through here.
    //
    // The check is on the *service*, not only the project: a project that
    // arrived from elsewhere holds only foreign services, but reading it
    // off the row that is actually being changed is the one that cannot be
    // wrong.
    let found = found.filter(|service| service.is_ours());

    let back = format!("/projects/{}", project.slug);
    Ok(found.map(|service| (project, service, back)))
}

/// Was a checkbox ticked?
///
/// A browser omits an unchecked box entirely rather than sending it
/// with a false value, so presence *is* the answer.
fn checked(form: &std::collections::HashMap<String, String>, name: &str) -> bool {
    form.contains_key(name)
}

/// The project and service a `/projects/…/services/…/…` path names.
///
/// `#[raw]` hands over the whole request and no extracted parameters,
/// so both slugs come out of the URI here.
fn service_path(path: &str) -> Option<(&str, &str)> {
    match crate::shell::auth::segments(path).as_slice() {
        // The service's own page, and every action under it — the
        // trailing parts differ (`deploy`, `ports`, `ports/<id>/delete`)
        // and none of them change which service is meant.
        ["projects", project, "services", service, ..]
            if !project.is_empty() && !service.is_empty() =>
        {
            Some((project, service))
        }
        _ => None,
    }
}

/// An optional port from a form field.
///
/// Empty is `None` — a service that serves no traffic is a real thing.
/// Anything else has to be a port, and `0` is not one.
fn parse_port(value: &str) -> Result<Option<u16>, &'static str> {
    if value.is_empty() {
        return Ok(None);
    }
    match value.parse::<u16>() {
        Ok(0) | Err(_) => Err("a container port is a number between 1 and 65535"),
        Ok(port) => Ok(Some(port)),
    }
}

/// `KEY=value` lines into pairs.
///
/// The split is on the *first* `=` only: a value is very often a URL or
/// a base64 blob with one in it, and splitting on every `=` truncates
/// exactly the secrets somebody would least like truncated.
///
/// Blank lines and `#` comments are skipped, so a block pasted out of
/// a `.env` file works as pasted.
fn parse_env(text: &str) -> Result<Vec<(String, String)>, String> {
    let mut pairs = Vec::new();

    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(format!("line {} is not KEY=value: {line:?}", number + 1));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("line {} has no name before the =", number + 1));
        }
        if !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
        {
            return Err(format!(
                "{key:?} is not a usable variable name — letters, digits, _ and ."
            ));
        }

        // The value is trimmed but not unquoted: `KEY="v"` in a shell
        // file means `v`, and guessing wrong either way puts literal
        // quotes into a password. Whoever pastes it can see the quotes.
        pairs.push((key.to_string(), value.trim().to_string()));
    }

    Ok(pairs)
}

/// Where to send the browser after a control was pressed.
///
/// The form says, because only the form knows: the same play and stop
/// are on the project's list and on the service's own page, and sending
/// somebody back to the list from the page they were reading is the
/// console deciding they meant to leave.
///
/// A path this console put in its own markup, never a `Referer` — that
/// would let a page elsewhere choose where a submit lands. Anything
/// that is not a path falls back to where it always went.
async fn returning_to(request: Request, fallback: &str) -> String {
    let form = match read_form(request).await {
        Ok(form) => form,
        Err(_) => return fallback.to_string(),
    };
    local_path(field(&form, "from"))
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::services::service_push_component::Push;
    use crate::infra::repositories::ports;
    use crate::web::console::tests::Console;
    use hypertext::Renderable;
    use wabot::rest::axum::http::StatusCode;

    /// The three answers a copy with no address can have, and the two the
    /// page used to collapse into one word.
    ///
    /// Jorge stopped a database and the copy on the other node read
    /// "waiting for that node". Nobody was waiting: that node had said
    /// what it had done and the page had no word for it — while the copies
    /// on this machine, in the same state, correctly said "not running".
    #[test]
    fn a_copy_that_is_not_meant_to_run_is_not_a_copy_we_are_waiting_for() {
        let replica = crate::domain::entities::replica::Replica {
            id: "rp-1".into(),
            service_id: "svc-1".into(),
            node_id: Some("nd-far".into()),
            slot: 3,
            address: None,
            overlay_port: Some(30001),
            last_error: None,
            evicted_at: None,
            reserved_host: None,
            memory_bytes: None,
            disk_bytes: None,
            cpu_millicores: None,
            following: None,
            wal_held: None,
            placed_by: None,
            placed_note: None,
        };

        // Through the renderer, which reads the one decision the stream
        // also sends — that pairing is what broke: this word was corrected
        // in the renderer and not in the decision, so the page said one
        // thing on load and the other two seconds later.
        let stopped = crate::features::services::service_placement_component::placement_state(
            &replica, true, false, false,
        )
        .render()
        .into_inner();
        assert!(stopped.contains("Not running"), "{stopped}");
        assert!(
            !stopped.contains("Waiting"),
            "a stopped copy is not one we are waiting on: {stopped}"
        );

        // Told, and the instruction not yet collected. Legible rather than
        // silent, which is most of what makes a wait feel long.
        let queued = crate::features::services::service_placement_component::placement_state(
            &replica, false, true, false,
        )
        .render()
        .into_inner();
        assert!(queued.contains("Queued for that node"), "{queued}");

        // And the case the word was written for: it has the instruction
        // and the answer has not come back.
        let waiting = crate::features::services::service_placement_component::placement_state(
            &replica, false, false, false,
        )
        .render()
        .into_inner();
        assert!(waiting.contains("Waiting for that node"), "{waiting}");
    }

    /// What a service offers, seen at a glance rather than one at a time.
    ///
    /// Two things Jorge corrected, and both were wrong in my model:
    ///
    /// - A service can be reached **several ways at once**. Each port is
    ///   its own exposure and a port can be published as raw TCP, served
    ///   over HTTPS at a hostname, or both — so a picker showing one and
    ///   hiding the rest answers the opposite of "what does this offer".
    /// - The inside name **does not depend on a port**. Every container in
    ///   the project resolves `<service>.<project>`, written from the
    ///   replica addresses; nothing about it reads the port table. The
    ///   first version gave a worker nothing at all, which is the case
    ///   where somebody most needs to be told the name.
    /// The settings page's badges are right when it renders, and it hosts
    /// no stream.
    ///
    /// It briefly hosted the project's live stream so two badges would
    /// update in place — `Asked` beside a name already served, and
    /// `Certificate on the way` after the certificate arrived. Navigating
    /// to and from this page then stopped working: the URL changed and the
    /// view did not. Reported by Jorge within minutes of it shipping, and
    /// **reverted on that evidence rather than on a proven cause** — the
    /// detail page hosts the same island safely, so what breaks when two
    /// views both host one is still unexplained.
    ///
    /// It costs little, because both badges are computed on the server:
    /// the edges one through the same `edge_cell` the stream uses, and the
    /// certificate one from whether the name is secured. This page is
    /// correct when it loads and a reload is how a change shows — which is
    /// a smaller wrong than a page you cannot leave.
    #[tokio::test]
    async fn the_settings_page_hosts_no_stream() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "one")])
            .send()
            .await;
        console
            .harness
            .post("/projects/one/services")
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;

        let page = console
            .harness
            .get("/projects/one/services/web/settings")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            !page.contains("project-live"),
            "the settings page hosts the stream again, which broke navigating away from it"
        );
    }

    #[tokio::test]
    async fn a_service_shows_every_way_in_and_always_its_own_name() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shop")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "web",
            "registry.example/web@sha256:abc",
            &[],
        )
        .await
        .expect("service");

        // A worker: no port, nothing outside, and still a name.
        let alone = crate::features::services::service_ports_component::exposure(
            &service,
            "shop",
            &[],
            Some("node.example"),
        );
        assert!(alone.outside.is_empty());
        assert_eq!(alone.inside, ["web.shop"]);

        // One port, published *and* named: two ways in, not a choice
        // between them.
        let http =
            crate::infra::repositories::ports::create(&database, &service.id, 80, true, None, &[])
                .await
                .expect("port");
        crate::infra::repositories::ports::set_hostname(
            &database,
            &http.id,
            Some("shop.example.com"),
        )
        .await
        .expect("named");
        // And a second port, exposed to nobody.
        crate::infra::repositories::ports::create(&database, &service.id, 9000, false, None, &[])
            .await
            .expect("port");

        let ports = crate::infra::repositories::ports::of_service(&database, &service.id)
            .await
            .expect("ports");
        let open = crate::features::services::service_ports_component::exposure(
            &service,
            "shop",
            &ports,
            Some("node.example"),
        );
        assert_eq!(
            open.outside.len(),
            2,
            "https and the raw port: {:?}",
            open.outside
        );
        assert!(open
            .outside
            .contains(&"https://shop.example.com".to_string()));
        assert!(open
            .outside
            .iter()
            .any(|url| url.starts_with("node.example:")));
        assert_eq!(
            open.inside,
            ["web.shop:80", "web.shop:9000"],
            "a name per port, including the one nobody outside can reach"
        );

        // Every string on screen at once, which is the whole point.
        let card = crate::features::services::service_ports_component::exposure_card(&open, true)
            .render()
            .into_inner();
        for url in open.outside.iter().chain(open.inside.iter()) {
            assert!(card.contains(url.as_str()), "{url} is missing: {card}");
        }
    }

    /// A database gets none of this, because it would be a lie.
    ///
    /// The block turns a port with a hostname into `https://<name>`, and a
    /// database answers the Postgres protocol on a TCP socket — the one
    /// thing an edge cannot proxy, which is why a database has no edge to
    /// choose in the first place. It shipped that way for one deploy and
    /// Jorge caught it on the page.
    #[tokio::test]
    async fn a_database_is_not_offered_an_https_url_it_does_not_serve() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let project = project(&console, &cookie).await;

        console
            .harness
            .post(&format!("/projects/{project}/databases"))
            .header("cookie", &cookie)
            .form(&[
                ("name", "orders"),
                ("postgres_version", "17"),
                ("memory", "67108864"),
            ])
            .send()
            .await;

        let body = console
            .harness
            .get(&format!("/projects/{project}/services/orders"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            !body.contains("https://orders"),
            "a database does not answer HTTPS anywhere: {body}"
        );
    }

    /// The first paint and the stream say the same thing, because they read
    /// the same decision.
    ///
    /// They did not: the word for a stopped copy elsewhere was corrected in
    /// the renderer and not in the cell the stream sends, so the row read
    /// "not running" on load and "waiting for that node" two seconds later.
    /// The comment above `replica_cell` had named that exact failure as the
    /// reason the two were kept together, which is not the same as making
    /// them one.
    #[test]
    fn what_the_page_paints_is_what_the_stream_sends() {
        let replica = crate::domain::entities::replica::Replica {
            id: "rp-1".into(),
            service_id: "svc-1".into(),
            node_id: Some("nd-far".into()),
            slot: 3,
            address: None,
            overlay_port: Some(30001),
            last_error: None,
            evicted_at: None,
            reserved_host: None,
            memory_bytes: None,
            disk_bytes: None,
            cpu_millicores: None,
            following: None,
            wal_held: None,
            placed_by: None,
            placed_note: None,
        };

        // Every combination, including the input added last — a state
        // left out of this list is a state where the two are free to
        // drift, which is the whole failure above.
        //
        // With an address and without: "not answering" is only reachable
        // for a copy that reported one, because the question it answers
        // is about a process that is up and not replying.
        for address in [None, Some("10.42.1.5".to_string())] {
            let replica = crate::domain::entities::replica::Replica {
                address,
                ..replica.clone()
            };
            for stopped in [true, false] {
                for queued in [true, false] {
                    for silent in [true, false] {
                        let painted = crate::features::services::service_placement_component::placement_state(
                            &replica, stopped, queued, silent,
                        )
                        .render()
                        .into_inner();
                        let streamed =
                            crate::features::services::project_cells_component::replica_cell(
                                &replica, stopped, queued, silent,
                            );
                        assert!(
                            painted.contains(&streamed.word),
                            "the paint says something the stream does not: \
                             {painted} vs {}",
                            streamed.word
                        );
                    }
                }
            }
        }
    }

    /// A project and a service to place, made the way the console does.
    async fn placed_service(
        console: &Console,
        cookie: &str,
    ) -> crate::domain::entities::service::Service {
        console
            .harness
            .post("/projects")
            .header("cookie", cookie)
            .form(&[("name", "shared")])
            .send()
            .await;
        let project = crate::infra::repositories::projects::all(&console.database)
            .await
            .expect("projects")
            .into_iter()
            .find(|project| project.slug == "shared")
            .expect("made");
        crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "web",
            "hub.example.com/shared/web@sha256:abc",
            &[],
        )
        .await
        .expect("service")
    }

    /// The one answer the console never had: what is it *saying*. Until
    /// this page, the only output anywhere was the single line kept on
    /// the row after a container died.
    ///
    /// Rendered into the page, not fetched: this works with scripting
    /// off, which matters most here because somebody opens a log when a
    /// node is unwell.
    #[tokio::test]
    async fn the_log_page_renders_the_copy_that_runs_here() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::infra::repositories::replicas::ensure_here(&console.database, &service.id, 1)
            .await
            .expect("a copy here");

        let page = console
            .ui
            .with_header("cookie", cookie)
            .get("/projects/shared/services/web/logs")
            .await;
        let html = page.html();

        assert!(html.contains("data-logs-out"), "no panel: {html}");
        assert!(
            html.contains("data-logs-state"),
            "nothing says whether it is following: {html}"
        );
        assert!(page.has_island("logs-live"), "{html}");
    }

    /// `?slot=2` answered with a page of JSON: a query parameter is
    /// text, the framework hands the field exactly what was in the URL,
    /// and a `u32` there refused it — `invalid type: string "2",
    /// expected u32`. The console's only numeric query parameter, which
    /// is why nothing had found it.
    ///
    /// And a number that names no copy here shows the first one rather
    /// than refusing: a stale link should show a log.
    #[tokio::test]
    async fn a_copy_can_be_chosen_by_number_in_the_url() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::infra::repositories::replicas::ensure_here(&console.database, &service.id, 2)
            .await
            .expect("two copies here");

        for (slot, wanted) in [("2", "2"), ("1", "1")] {
            let body = console
                .harness
                .get(&format!("/projects/shared/services/web/logs?slot={slot}"))
                .header("cookie", &cookie)
                .send()
                .await
                .body;
            assert!(!body.contains("invalid type"), "{body}");
            assert!(
                body.contains(&format!("data-slot=\"{wanted}\"")),
                "asked for copy {slot} and got: {body}"
            );
        }

        for nonsense in ["9", "not-a-number", ""] {
            let body = console
                .harness
                .get(&format!(
                    "/projects/shared/services/web/logs?slot={nonsense}"
                ))
                .header("cookie", &cookie)
                .send()
                .await
                .body;
            assert!(
                body.contains("data-slot=\"1\""),
                "{nonsense:?} did not fall back to the first copy: {body}"
            );
        }
    }

    /// A copy elsewhere writes its log on the machine that runs it, and
    /// this node asks that machine over the overlay — which is what
    /// phase 9's channel was built for, and what this page said it could
    /// not do for three phases.
    ///
    /// What is asserted here is the half a test can reach: the reading
    /// is attempted, and when it cannot be had the reason is on the page
    /// rather than in the journal. That it *arrives* is a two-node
    /// question — in a test both ends are one database — and it is
    /// verified on the nodes.
    #[tokio::test]
    async fn a_log_that_lives_on_another_node_is_asked_for_over_the_network() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        // The copy this service already has, sent away — which is the
        // real shape of it: a service with one copy, running elsewhere.
        let here = crate::infra::repositories::replicas::of_service(&console.database, &service.id)
            .await
            .expect("copies");
        for replica in &here {
            crate::infra::repositories::replicas::move_to(
                &console.database,
                &replica.id,
                Some("nd-elsewhere"),
            )
            .await
            .expect("moved");
        }

        let page = console
            .ui
            .with_header("cookie", cookie)
            .get("/projects/shared/services/web/logs")
            .await;
        let html = page.html();

        // Nothing to follow: the island polls this node's own endpoint,
        // and there is no copy of this here to poll for.
        assert!(!html.contains("data-logs-out"), "an empty panel: {html}");
        // And the reason it could not be read is *on the page*. In this
        // harness the holder is a node with no row, which is one of the
        // several ways the answer does not arrive — every one of them
        // ends up here rather than in the journal.
        assert!(
            html.contains("no row for the one holding that copy"),
            "the page has to say why it could not ask: {html}"
        );
    }

    /// A slot number in a query string must not be a way to read a
    /// container this service does not have — the id is built from the
    /// project, the service and the slot, so an unchecked slot would
    /// name somebody else's container.
    ///
    /// Refusals rather than a successful stream, deliberately: a stream
    /// that works never ends, so a test that asked for one would wait
    /// for a body that never comes. The reading itself is covered by
    /// `deploy::logs`.
    #[tokio::test]
    async fn a_log_stream_refuses_a_copy_that_does_not_run_here() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::infra::repositories::replicas::ensure_here(&console.database, &service.id, 1)
            .await
            .expect("a copy here");

        // A slot that does not exist, and a slot number of zero.
        for slot in ["7", "0"] {
            console
                .harness
                .get(&format!(
                    "/projects/shared/services/web/logs/live?slot={slot}&from=0"
                ))
                .header("cookie", &cookie)
                .send()
                .await
                .assert_status(StatusCode::NOT_FOUND);
        }
    }

    /// And it is not a way past the session either.
    #[tokio::test]
    async fn a_log_stream_refuses_a_stranger() {
        let console = Console::new().await;
        console
            .harness
            .get("/projects/shared/services/web/logs/live?slot=1&from=0")
            .send()
            .await
            .assert_status(StatusCode::UNAUTHORIZED);
    }

    /// The page you go to when a service is down was the one page you
    /// could not start it from: it showed the state and withheld the
    /// control.
    #[tokio::test]
    async fn the_service_page_offers_the_control_its_state_calls_for() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        placed_service(&console, &cookie).await;

        let body = console
            .harness
            .get("/projects/shared/services/web")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("/projects/shared/services/web/deploy"),
            "no way to start it: {body}"
        );
        assert!(
            body.contains("/projects/shared/services/web/stop"),
            "{body}"
        );
    }

    /// The badge said "Deploying" until somebody reloaded, on the one
    /// page they were watching to find out when it stopped. It joins
    /// the project's stream rather than opening one of its own: the
    /// island writes by service id, and this page has one of each cell.
    #[tokio::test]
    async fn the_service_page_comes_alive_on_the_projects_stream() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        let page = console
            .ui
            .with_header("cookie", cookie)
            .get("/projects/shared/services/web")
            .await;

        assert!(page.has_island("project-live"), "{}", page.html());
        assert_eq!(
            page.island_props("project-live"),
            Some(serde_json::json!({ "project": "shared" })),
            "the client needs the project to open the stream"
        );

        let html = page.html();
        for hook in [
            format!("data-state=\"{}\"", service.id),
            format!("data-address=\"{}\"", service.id),
        ] {
            assert!(
                html.contains(&hook),
                "nothing for the stream to write into: {hook}"
            );
        }
    }

    /// The same two controls are on the list and on the service's own
    /// page. Sending somebody back to the list from the page they were
    /// reading is the console deciding they meant to leave.
    #[tokio::test]
    async fn a_control_returns_to_the_page_it_was_pressed_on() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        placed_service(&console, &cookie).await;

        let response = console
            .harness
            .post("/projects/shared/services/web/stop")
            .header("cookie", &cookie)
            .form(&[("from", "/projects/shared/services/web")])
            .send()
            .await;
        // The reason rides along when there is one — no containerd in
        // a test — but the *page* is what this pins.
        let location = response.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/projects/shared/services/web"),
            "{location}"
        );

        // A path this console put in its own markup, never somewhere
        // else's: that would let another page choose where a submit
        // lands.
        let elsewhere = console
            .harness
            .post("/projects/shared/services/web/stop")
            .header("cookie", &cookie)
            .form(&[("from", "https://elsewhere.example/")])
            .send()
            .await;
        let location = elsewhere.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/projects/shared?") || location == "/projects/shared",
            "somewhere else chose where the submit landed: {location}"
        );
    }

    /// By slot number alone, lowering a service with a dead #2 and a
    /// healthy #3 removes the healthy one and keeps the corpse.
    #[tokio::test]
    async fn lowering_the_count_lets_the_evicted_go_first() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        // #2 thrown out where it ran, #3 alive and here.
        let two = crate::infra::repositories::replicas::place(
            &console.database,
            &service.id,
            Some("nd-elsewhere1"),
            2,
        )
        .await
        .expect("placed");
        crate::infra::repositories::replicas::evict(&console.database, &two.id)
            .await
            .expect("evicted");
        crate::infra::repositories::replicas::place(&console.database, &service.id, None, 3)
            .await
            .expect("placed");

        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "2")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let left: Vec<u32> =
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas")
                .into_iter()
                .map(|replica| replica.slot)
                .collect();
        assert_eq!(
            left,
            vec![1, 3],
            "the corpse was kept and the live one killed"
        );
    }

    /// A service placed here by another node is not this node's to
    /// start or stop — the same boundary every mutation goes through,
    /// and the page must not offer what the POST will refuse.
    #[tokio::test]
    async fn a_foreign_service_is_offered_no_controls() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        services::set_origin(&console.database, &service.id, "nd-elsewhere1")
            .await
            .expect("origin");

        let body = console
            .harness
            .get("/projects/shared/services/web")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            !body.contains("/projects/shared/services/web/deploy"),
            "it offered to start somebody else's service"
        );
        assert!(body.contains("From another node"), "{body}");
    }

    /// The one thing the operator of a machine can always do to
    /// something they did not put there. The machine is theirs even
    /// when the orders are not.
    #[tokio::test]
    async fn a_service_from_another_node_can_be_thrown_off_this_one() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        services::set_origin(&console.database, &service.id, "nd-elsewhere1")
            .await
            .expect("origin");

        console
            .harness
            .post("/projects/shared/services/web/evict")
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let placements =
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas");
        assert!(
            placements.iter().all(|replica| replica.evicted()),
            "{placements:?}"
        );

        // The rows stay: they are what the next report carries, and
        // that report is the only thing that makes the other node stop
        // asking.
        assert!(
            services::in_project(&console.database, &service.project_id, "web")
                .await
                .expect("query")
                .is_some(),
            "the tombstone was removed, so the errand will come back"
        );
    }

    /// The guard is the inverse of `locate`'s: a service this node made
    /// itself is *deleted*, not evicted, and the two buttons do not
    /// mean the same thing.
    #[tokio::test]
    async fn a_service_of_this_nodes_own_is_not_evictable() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        let response = console
            .harness
            .post("/projects/shared/services/web/evict")
            .header("cookie", &cookie)
            .send()
            .await;

        let location = response.header("location").unwrap_or_default();
        assert!(location.contains("error="), "{location}");
        assert!(
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas")
                .iter()
                .all(|replica| !replica.evicted()),
            "somebody's own service was evicted"
        );
    }

    /// Throwing another node's work off this machine is the node's
    /// business, not one project's.
    #[tokio::test]
    async fn only_an_admin_may_evict() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let service = placed_service(&console, &admin).await;
        services::set_origin(&console.database, &service.id, "nd-elsewhere1")
            .await
            .expect("origin");
        let member = console.joined_as(&admin, "member").await;

        let response = console
            .harness
            .post("/projects/shared/services/web/evict")
            .header("cookie", &member)
            .send()
            .await;

        assert_eq!(response.header("location"), Some("/"));
        assert!(
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas")
                .iter()
                .all(|replica| !replica.evicted()),
            "a member evicted another node's service"
        );
    }

    /// The page the whole goal hangs off: a service is administered
    /// from the node that created it, and that is where somebody says
    /// how many copies run and where each of them goes.
    #[tokio::test]
    async fn a_replica_is_placed_on_another_node_from_the_service_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-elsewhere1".into(),
                name: "alpine.example".into(),
                kind: crate::infra::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("node");

        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "2")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[
                ("replicas", "2"),
                ("slot-1", ""),
                ("slot-2", "nd-elsewhere1"),
            ])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let placements =
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas");
        assert_eq!(placements.len(), 2);
        assert!(placements[0].is_here());
        assert_eq!(placements[1].node_id.as_deref(), Some("nd-elsewhere1"));

        // And that node has been asked to run it.
        let queued = crate::infra::network::errand::all(&console.database)
            .await
            .expect("errands");
        assert_eq!(queued.len(), 1, "{queued:?}");
        assert_eq!(queued[0].node_id, "nd-elsewhere1");

        let waiting = crate::infra::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("waiting");
        let host: crate::infra::network::errand::Host =
            serde_json::from_value(waiting[0].payload.clone()).expect("a host errand");
        assert_eq!(host.service, "web");
        // And with no credential, because the image names a registry
        // that is not this node's. A token minted here means nothing
        // there, and sending one to `docker.io` would be handing a
        // wabot push token to Docker Hub — which is the ordinary case
        // for a managed database, whose image comes from exactly there.
        assert!(
            host.secret.is_none(),
            "a credential for this node's registry was sent to {}",
            host.registry
        );
    }

    /// "Wherever there is room" spreads the copies and says why on the
    /// row.
    ///
    /// The end of the path the planner's own tests cannot reach: they
    /// are pure and take candidates as given, and what this pins is that
    /// the gathering half hands them the right ones and that what comes
    /// back becomes an ordinary `replica`.
    ///
    /// **Two copies on two machines**, not two on the emptiest — the
    /// spread is what a second copy is for. And each carries a sentence,
    /// because a decision nobody can read is one nobody will run in
    /// production.
    #[tokio::test]
    async fn copies_placed_anywhere_are_spread_and_say_why() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        for (id, name, memory) in [
            ("nd-roomy0001", "ubuntu", 900u64),
            ("nd-tight0001", "alpine", 400u64),
        ] {
            crate::infra::network::save(
                &console.database,
                &crate::infra::network::Node {
                    update_note: None,
                    version: None,
                    wanted_version: None,
                    usage: None,
                    draining_at: None,
                    capacity: Some(crate::infra::network::Capacity {
                        memory_allocatable: memory * 1024 * 1024,
                        memory_committed: 0,
                        cpu_allocatable: 4_000,
                        cpu_committed: 0,
                        unbounded: 0,
                    }),
                    id: id.into(),
                    name: name.into(),
                    kind: crate::infra::network::Kind::Private,
                    endpoint: None,
                    public_key: Some("k".into()),
                    overlay_ip: Some(format!("10.42.0.{}", memory % 200)),
                    is_self: false,
                    last_seen_at: None,
                    allows: vec![crate::domain::values::capability::Capability::Host],
                    ca_pem: None,
                },
            )
            .await
            .expect("node");
        }

        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "3"), ("new-on", ANYWHERE)])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let placements =
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas");
        assert_eq!(placements.len(), 3);

        // Slot 1 was already here and placed by a person; the two the
        // node added are the planner's, on the two other machines.
        let added: Vec<&crate::domain::entities::replica::Replica> = placements
            .iter()
            .filter(|replica| {
                replica.placed_by == Some(crate::domain::entities::replica::Placed::Planner)
            })
            .collect();
        assert_eq!(added.len(), 2, "{placements:?}");

        let mut on: Vec<&str> = added
            .iter()
            .filter_map(|replica| replica.node_id.as_deref())
            .collect();
        on.sort_unstable();
        assert_eq!(
            on,
            vec!["nd-roomy0001", "nd-tight0001"],
            "two copies landed on one machine"
        );

        for replica in &added {
            let why = replica.placed_note.as_deref().unwrap_or_default();
            assert!(!why.is_empty(), "a placement nobody can read: {replica:?}");
            assert!(why.contains("free"), "{why}");
        }

        // And both machines were told, which is the half a placement
        // written straight into the table would have skipped.
        for node in ["nd-roomy0001", "nd-tight0001"] {
            assert!(
                !crate::infra::network::errand::waiting(&console.database, node)
                    .await
                    .expect("waiting")
                    .is_empty(),
                "{node} was never told to run its copy"
            );
        }
    }

    /// Draining a node moves what this one put there, and says what it
    /// could not move.
    ///
    /// **The operation every upgrade and every hardware failure needs.**
    /// What it reports is rows moved *here*, never containers stopped
    /// there: the far node stops when it next collects, which for a
    /// machine that is down is whenever it comes back — so a console
    /// claiming "drained" would be claiming something it cannot know.
    #[tokio::test]
    async fn draining_a_node_moves_what_this_one_put_there() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        // This node takes no copies, so the other machine is the only
        // place left. Said rather than assumed: on Linux this node
        // measures its own memory and is the emptiest candidate, so the
        // copy came home instead — the test passed only where the
        // machine running it cannot measure itself, which is a Mac, and
        // failed on every Linux CI run.
        crate::infra::network::capability::set_provides(
            &console.database,
            crate::domain::values::capability::Capability::Host,
            false,
        )
        .await
        .expect("this node takes no copies");

        for (id, name) in [("nd-going0001", "leaving"), ("nd-staying001", "staying")] {
            crate::infra::network::save(
                &console.database,
                &crate::infra::network::Node {
                    update_note: None,
                    version: None,
                    wanted_version: None,
                    usage: None,
                    draining_at: None,
                    capacity: Some(crate::infra::network::Capacity {
                        memory_allocatable: 900 * 1024 * 1024,
                        memory_committed: 0,
                        cpu_allocatable: 4_000,
                        cpu_committed: 0,
                        unbounded: 0,
                    }),
                    id: id.into(),
                    name: name.into(),
                    kind: crate::infra::network::Kind::Private,
                    endpoint: None,
                    public_key: Some("k".into()),
                    overlay_ip: Some(format!("10.42.0.{}", id.len())),
                    is_self: false,
                    last_seen_at: None,
                    allows: vec![crate::domain::values::capability::Capability::Host],
                    ca_pem: None,
                },
            )
            .await
            .expect("node");
        }

        // One copy on the machine that is about to leave.
        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[
                ("replicas", "1"),
                ("slot-1", "nd-going0001"),
                ("new-on", ""),
            ])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        console
            .harness
            .post("/network/nd-going0001/drain")
            .header("cookie", &cookie)
            .form(&[("draining", "on")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let placements =
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas");
        assert_eq!(
            placements[0].node_id.as_deref(),
            Some("nd-staying001"),
            "the copy stayed on the machine being emptied"
        );
        let why = placements[0].placed_note.as_deref().unwrap_or_default();
        assert!(why.contains("moved off nd-going0001"), "{why}");

        // The mark is a state, not an event: it outlives the press, so
        // nothing chooses that machine again until somebody says so.
        assert!(
            crate::infra::network::find(&console.database, "nd-going0001")
                .await
                .expect("query")
                .expect("row")
                .draining_at
                .is_some(),
            "the drain did not outlive the button"
        );

        // And the machine being emptied was told it now runs none of
        // this — the instruction that is only ever built from the rows,
        // where it no longer appears.
        let told = crate::infra::network::errand::waiting(&console.database, "nd-going0001")
            .await
            .expect("waiting");
        let last: crate::infra::network::errand::Host =
            serde_json::from_value(told.last().expect("told").payload.clone())
                .expect("a host errand");
        assert!(
            last.slots.is_empty(),
            "the emptied node was not told it holds none of it: {last:?}"
        );
    }

    /// Putting it back in service moves nothing, and stops the mark.
    ///
    /// Draining and undraining are not symmetrical on purpose: what came
    /// off does not go back. A copy that moved is running somewhere now,
    /// and moving it a second time to undo a decision would be an outage
    /// nobody asked for — the mark is what changes, and the planner will
    /// choose the machine again when it next has something to place.
    #[tokio::test]
    async fn putting_a_node_back_in_service_moves_nothing() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: Some(1),
                capacity: None,
                id: "nd-back000001".into(),
                name: "back".into(),
                kind: crate::infra::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.12".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("node");

        console
            .harness
            .post("/network/nd-back000001/drain")
            .header("cookie", &cookie)
            .form(&[("draining", "off")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert_eq!(
            crate::infra::network::find(&console.database, "nd-back000001")
                .await
                .expect("query")
                .expect("row")
                .draining_at,
            None
        );
    }

    /// Nowhere is a refusal on the form, never a quiet copy here.
    ///
    /// The failure this guards is the one `ensure_count(.., None)` still
    /// has: no node named, so the copy lands on this machine whatever it
    /// has left. Under the planner that would be a node quietly
    /// over-committing itself with nothing having said so.
    #[tokio::test]
    async fn nowhere_to_put_a_copy_is_said_rather_than_placed_here() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        // A ceiling no node could hold, and no other node on the
        // network at all — so the only candidate is this one, whose
        // meters read nothing on the machine the suite runs on.
        crate::infra::repositories::services::set_memory_limit(
            &console.database,
            &service.id,
            Some(64 * 1024 * 1024),
        )
        .await
        .expect("ceiling");
        crate::infra::network::capability::set_provides(
            &console.database,
            crate::domain::values::capability::Capability::Host,
            false,
        )
        .await
        .expect("this node runs nothing for anybody");

        let response = console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "2"), ("new-on", ANYWHERE)])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").unwrap_or_default().to_string();
        assert!(location.contains("error="), "{location}");
        assert!(location.contains("nowhere"), "{location}");

        assert_eq!(
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas")
                .len(),
            1,
            "a copy was placed here anyway"
        );
    }

    /// Raising a ceiling reaches the machines running the copies.
    ///
    /// The derived instruction is built from a **fresh** read of the
    /// row, not from the struct the caller was holding — the trap
    /// `running` fell into and was fixed for, in the same place and for
    /// the same reason: the resources form writes the number and then
    /// asks for the dispatch, so a captured struct still carries the
    /// value the operator had just changed away from. That reads as a
    /// form that saved and did nothing.
    #[tokio::test]
    async fn a_ceiling_travels_to_the_node_running_the_copy() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-elsewhere1".into(),
                name: "alpine.example".into(),
                kind: crate::infra::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
                allows: vec![crate::domain::values::capability::Capability::Host],
                ca_pem: None,
            },
        )
        .await
        .expect("node");

        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "1"), ("slot-1", "nd-elsewhere1")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let ceiling = |payload: &serde_json::Value| {
            serde_json::from_value::<crate::infra::network::errand::Host>(payload.clone())
                .expect("a host errand")
                .memory_limit
        };
        // Placed with none, because the service has none.
        let placed = crate::infra::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("waiting");
        assert_eq!(ceiling(&placed[0].payload), None);

        console
            .harness
            .post("/projects/shared/services/web/memory")
            .header("cookie", &cookie)
            .form(&[("memory", "256 MB"), ("cpu", "")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert_eq!(
            crate::infra::repositories::services::find(&console.database, &service.id)
                .await
                .expect("service")
                .expect("row")
                .memory_limit,
            Some(256 * 1024 * 1024),
            "the form did not write it, so the rest of this proves nothing"
        );

        let after = crate::infra::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("waiting");
        let latest = after.last().expect("an instruction after the change");
        assert_eq!(
            ceiling(&latest.payload),
            Some(256 * 1024 * 1024),
            "the node running the copy was told the old number"
        );
    }

    /// The other half of the rule: an image in **this node's own**
    /// registry does travel with something to pull it, or the far node
    /// gets a 401 instead of a container.
    #[tokio::test]
    async fn an_image_in_this_nodes_registry_travels_with_a_credential() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        crate::infra::node::settings::set_domain(&console.database, Some("hub.example"))
            .await
            .expect("domain");

        let project = crate::infra::repositories::projects::create(&console.database, "shared")
            .await
            .expect("project");
        crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "web",
            "hub.example/shared/web:latest",
            &[],
        )
        .await
        .expect("service");
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-elsewhere2".into(),
                name: "alpine.example".into(),
                kind: crate::infra::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("node");

        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "2"), ("new-on", "nd-elsewhere2")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let waiting = crate::infra::network::errand::waiting(&console.database, "nd-elsewhere2")
            .await
            .expect("waiting");
        let host: crate::infra::network::errand::Host =
            serde_json::from_value(waiting[0].payload.clone()).expect("a host errand");
        assert!(host.secret.is_some(), "nothing to pull it with");
        assert_eq!(host.username.as_deref(), Some("errand"));
    }

    /// Bringing a replica home has to tell the node it left.
    ///
    /// This shipped in v0.5.0 as a leak: the row moved and nothing was
    /// sent, so that node kept the container running for ever — and a
    /// report about it is ignored by design, because the row is local
    /// now. Invisible from both sides, which is the worst shape a bug
    /// can have here.
    #[tokio::test]
    async fn bringing_a_replica_home_tells_the_node_it_left() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-elsewhere1".into(),
                name: "alpine.example".into(),
                kind: crate::infra::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("node");

        // Two replicas, the second one away.
        for form in [
            vec![("replicas", "2")],
            vec![("replicas", "2"), ("slot-2", "nd-elsewhere1")],
        ] {
            console
                .harness
                .post("/projects/shared/services/web/placement")
                .header("cookie", &cookie)
                .form(&form)
                .send()
                .await
                .assert_status(StatusCode::SEE_OTHER);
        }
        crate::infra::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("collected");

        // And back home.
        console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "2"), ("slot-1", ""), ("slot-2", "")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let placements =
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas");
        assert!(placements.iter().all(|replica| replica.is_here()));

        // The node it left is told, and told it runs *none* of it —
        // which is the instruction that stops its container.
        // The newest one, and it says it runs *none* of this — which
        // is the instruction that stops its container. Collecting does
        // not settle an errand, so the earlier one is still pending
        // beside it; what matters is the last thing it was told.
        let waiting = crate::infra::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("waiting");
        let latest = waiting.last().expect("the node it left was told nothing");
        let host: crate::infra::network::errand::Host =
            serde_json::from_value(latest.payload.clone()).expect("a host errand");
        assert!(
            host.slots.is_empty(),
            "it was told to keep running {:?}",
            host.slots
        );
    }

    /// Dropping a replica that runs somewhere else used to be refused,
    /// because nothing could tell that node to stop and the row going
    /// would have left its container running with nothing naming it.
    /// A `host` errand now carries the slots that are *left*, and an
    /// empty list is a real instruction — so the row can go, and the
    /// node it ran on finds out on its next poll.
    #[tokio::test]
    async fn dropping_a_replica_that_runs_elsewhere_tells_that_node() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::infra::repositories::replicas::place(
            &console.database,
            &service.id,
            Some("nd-elsewhere1"),
            2,
        )
        .await
        .expect("placed");

        let response = console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "1")])
            .send()
            .await;

        let location = response.header("location").unwrap_or_default();
        assert!(!location.contains("error="), "{location}");
        assert_eq!(
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("replicas")
                .len(),
            1,
            "the row it was told to drop is still there"
        );

        let latest = crate::infra::network::errand::waiting(&console.database, "nd-elsewhere1")
            .await
            .expect("errands")
            .pop()
            .expect("something was queued for it");
        let host: crate::infra::network::errand::Host =
            serde_json::from_value(latest.payload.clone()).expect("a host errand");
        assert!(
            host.slots.is_empty(),
            "it was told to keep running {:?}",
            host.slots
        );
    }

    /// A service that arrived on an errand is administered from the
    /// node that sent it, and this is the boundary rather than a hidden
    /// button: every mutation goes through `locate`, so a foreign
    /// service answers the way a stranger's does.
    ///
    /// This was claimed in the commit that added the guard and was not
    /// actually there — a text replacement whose anchor did not exist
    /// failed silently. Hence the test, and hence this note.
    #[tokio::test]
    async fn a_service_from_another_node_cannot_be_changed_here() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        services::set_origin(&console.database, &service.id, "nd-elsewhere1")
            .await
            .expect("origin");

        // Stopping is the smallest mutation there is, and it goes
        // through the same door as every other.
        let response = console
            .harness
            .post("/projects/shared/services/web/stop")
            .header("cookie", &cookie)
            .send()
            .await;
        // "No such service" is the answer on purpose: a foreign one
        // reads exactly the way a stranger's does, which is what makes
        // this a boundary rather than a hidden button.
        let location = response.header("location").unwrap_or_default();
        assert!(
            location.starts_with("/?error="),
            "a foreign service was changed from here: {location}"
        );

        let stored = services::in_project(&console.database, &service.project_id, "web")
            .await
            .expect("query")
            .expect("there");
        assert_eq!(
            stored.desired_state,
            crate::domain::entities::service::DesiredState::Running,
            "the intent was changed"
        );
    }

    async fn project(console: &Console, cookie: &str) -> String {
        console
            .harness
            .post("/projects")
            .header("cookie", cookie)
            .form(&[("name", "My API")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        "my-api".to_string()
    }

    /// The values a service was made with can be restored after the
    /// first edit.
    ///
    /// **Shipped otherwise**: creating a service recorded no revision, so
    /// the first edit left a history of one — what was running — and no
    /// way back. Found on a VM node; nothing tested restoring at all.
    #[tokio::test]
    async fn the_environment_a_service_was_made_with_can_be_restored() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;
        console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[
                ("name", "web"),
                ("image", "docker.io/library/nginx:alpine"),
                ("env", "GREETING=hello"),
            ])
            .send()
            .await;
        let base = format!("/projects/{slug}/services/web");
        console
            .harness
            .post(&format!("{base}/env"))
            .header("cookie", &cookie)
            .form(&[("env", "GREETING=bonjour")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let body = console
            .harness
            .get(&format!("{base}/settings/environment"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        let restore = body
            .split("action=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .find(|action| action.ends_with("/revert"))
            .unwrap_or_else(|| panic!("nothing to restore: {body}"))
            .to_string();

        console
            .harness
            .post(&restore)
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        let project = crate::infra::repositories::projects::find(&console.database, &slug)
            .await
            .expect("query")
            .expect("present");
        let service = services::in_project(&console.database, &project.id, "web")
            .await
            .expect("query")
            .expect("present");
        assert_eq!(
            service.env.get("GREETING").map(String::as_str),
            Some("hello")
        );
    }

    #[tokio::test]
    async fn a_service_is_created_and_listed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[
                ("name", "web"),
                ("image", "docker.io/library/nginx:alpine"),
                ("env", "LOG_LEVEL=info\nDSN=postgres://a=b"),
            ])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(
            response.header("location"),
            Some("/projects/my-api/services/web"),
            "straight to the service, where its ports are declared"
        );

        let body = console
            .harness
            .get("/projects/my-api")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("web"), "{body}");
        // Not the image: the list dropped that column. Which image a
        // service runs is on the service's own page, where somebody
        // deciding about it is already looking.
        assert!(!body.contains("nginx:alpine"), "{body}");

        // The value with an `=` in it survived the form.
        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        assert_eq!(
            stored.env.get("DSN").map(String::as_str),
            Some("postgres://a=b")
        );
    }

    /// A deployment that could not happen has to say so on the page,
    /// not just in a log. This runs where containerd is not — a
    /// developer's machine — which is exactly the failure an operator
    /// meets when the socket is down.
    ///
    /// The reason is put on the row by `Deployer::deploy`, which is
    /// what the job calls. Driven directly here rather than through the
    /// POST: creating a service now *queues* a deployment, and this
    /// harness has no job runner, so going through the form would test
    /// that nothing happened.
    #[tokio::test]
    async fn a_deployment_that_fails_says_why_on_the_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;

        console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        let found = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("present");

        let deployer = crate::infra::deploy::Deployer::new(
            console.database.clone(),
            &crate::infra::config::Config::default(),
        );
        let _ = deployer.deploy(&found, &stored).await;

        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        let failure =
            crate::infra::repositories::replicas::of_service(&console.database, &stored.id)
                .await
                .expect("replicas")
                .into_iter()
                .next()
                .and_then(|replica| replica.last_error)
                .expect("the reason was recorded");
        assert!(
            failure.contains("containerd"),
            "the reason names the runtime: {failure}"
        );
        assert!(
            crate::infra::repositories::replicas::of_service(&console.database, &stored.id)
                .await
                .expect("replicas")
                .into_iter()
                .all(|replica| replica.address.is_none()),
            "nothing to route to"
        );

        let body = console
            .harness
            .get("/projects/my-api")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("class=\"failure\""), "the row is rendered");
        assert!(body.contains("containerd"), "with the reason in it");
    }

    /// The join the node found broken: a queued deployment has to
    /// reach `Deployer::deploy` and leave its reason on the row.
    ///
    /// It did not, in production, because nothing registered
    /// `SqliteDatabase` in the container and the handler could not be
    /// built — the worker panicked, the button still answered, and the
    /// service stayed as it was. This is the test that would have
    /// caught it.
    #[tokio::test]
    async fn a_queued_deployment_reaches_the_row() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/deploy"))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        // Polled, not slept: the job runs on a spawned task, and a
        // fixed wait is either flaky or slow.
        let mut reason = None;
        for _ in 0..150 {
            let stored = services::all(&console.database, None)
                .await
                .expect("query")
                .pop()
                .expect("one service");
            if let Some(found) =
                crate::infra::repositories::replicas::of_service(&console.database, &stored.id)
                    .await
                    .expect("replicas")
                    .into_iter()
                    .next()
                    .and_then(|replica| replica.last_error)
            {
                reason = Some(found);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let reason = reason.expect("the handler ran and recorded a reason");
        assert!(
            reason.contains("containerd"),
            "which names the runtime: {reason}"
        );
    }

    /// The POST answers before the work happens. Holding the request
    /// open for an image pull is what offered somebody "confirm form
    /// A service is not deleted without its name typed.
    ///
    /// There was no confirmation: one press of the danger-zone button
    /// removed the service, its container and — for a database — every
    /// byte it had stored. Reported by Jorge, who pressed it.
    ///
    /// Posted with the wrong name as well as with none, because
    /// `required` in the markup is a courtesy the browser extends and a
    /// request can be constructed by hand.
    #[tokio::test]
    async fn a_service_is_not_deleted_without_its_name_typed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "one")])
            .send()
            .await;
        console
            .harness
            .post("/projects/one/services")
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;

        for body in [vec![], vec![("confirm", "not-web")]] {
            console
                .harness
                .post("/projects/one/services/web/delete")
                .header("cookie", &cookie)
                .form(&body)
                .send()
                .await;
            let page = console
                .harness
                .get("/projects/one")
                .header("cookie", &cookie)
                .send()
                .await
                .body;
            assert!(
                page.contains("web"),
                "the service survived a deletion nobody confirmed"
            );
        }
    }

    /// resubmission" when they reloaded.
    #[tokio::test]
    async fn deploying_answers_without_waiting_for_the_deployment() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("{page}/deploy"))
            .header("cookie", &cookie)
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);

        // The job is spawned, not awaited, so the row is still
        // untouched the instant the answer arrives. What eventually
        // lands there is the previous test's subject.
        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        assert!(
            crate::infra::repositories::replicas::of_service(&console.database, &stored.id)
                .await
                .expect("replicas")
                .into_iter()
                .all(|replica| replica.address.is_none()),
            "nothing was deployed inline"
        );
    }

    #[tokio::test]
    async fn a_bad_image_returns_to_the_form_with_the_reason() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "https://example.com/nginx")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").expect("redirected");
        assert!(
            location.starts_with("/projects/my-api/services/new?error="),
            "{location}"
        );
    }

    #[tokio::test]
    async fn creating_a_service_needs_a_session() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;
        assert_eq!(response.header("location"), Some("/sign-in"));
        assert!(services::all(&console.database, None)
            .await
            .expect("query")
            .is_empty());
    }

    #[tokio::test]
    async fn a_service_can_be_deleted_from_the_project_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let slug = project(&console, &cookie).await;
        console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", &cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;

        let response = console
            .harness
            .post(&format!("/projects/{slug}/services/web/delete"))
            .header("cookie", &cookie)
            .form(&[("confirm", "web")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/projects/my-api"));
        assert!(services::all(&console.database, None)
            .await
            .expect("query")
            .is_empty());
    }

    /// A service of the same name in another project must not be the
    /// one that disappears.
    #[tokio::test]
    async fn deleting_names_the_service_inside_its_own_project() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        for name in ["one", "two"] {
            console
                .harness
                .post("/projects")
                .header("cookie", &cookie)
                .form(&[("name", name)])
                .send()
                .await;
            console
                .harness
                .post(&format!("/projects/{name}/services"))
                .header("cookie", &cookie)
                .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
                .send()
                .await;
        }

        console
            .harness
            .post("/projects/one/services/web/delete")
            .header("cookie", &cookie)
            .form(&[("confirm", "web")])
            .send()
            .await;

        let left = services::all(&console.database, None).await.expect("query");
        assert_eq!(left.len(), 1, "only one went");
        let survivor = crate::infra::repositories::projects::find(&console.database, "two")
            .await
            .expect("query")
            .expect("present");
        assert_eq!(left[0].project_id, survivor.id, "and it was the right one");
    }

    /// The command has to be addressed the way the registry reads a
    /// push — host, name without one, and the tag this service watches
    /// — or it is three pages of guessing with a wrong answer at the
    /// end.
    ///
    /// And three services cannot be pushed to at all, which the card has
    /// to *say* rather than leave blank. Printing nothing was the first
    /// version, and it put an empty space under a line telling somebody
    /// to push an image to this repository — reported straight away,
    /// against a service tracking `docker.io/library/nginx`.
    #[tokio::test]
    async fn a_push_command_is_addressed_the_way_the_registry_reads_one() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");

        let ours = crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "api",
            "node.example.com/my-api/api:latest",
            &[],
        )
        .await
        .expect("service");
        assert!(matches!(
            crate::features::services::service_push_component::push(&ours, "my-api", Some("node.example.com")),
            Some(Push::To(target)) if target == "node.example.com/my-api/api:latest"
        ));
        assert!(
            matches!(
                crate::features::services::service_push_component::push(&ours, "my-api", None),
                Some(Push::Nameless)
            ),
            "a node with no domain has no host to write into a reference"
        );

        let pinned = crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "pinned",
            "node.example.com/my-api/pinned@sha256:abc",
            &[],
        )
        .await
        .expect("service");
        assert!(
            matches!(
                crate::features::services::service_push_component::push(
                    &pinned,
                    "my-api",
                    Some("node.example.com")
                ),
                Some(Push::Pinned)
            ),
            "a digest names bytes, and nothing watches it"
        );

        // Created with no image, waiting for its first push: the form
        // promises the commands, and the card said "pinned by digest".
        let waiting = crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "waiting",
            "",
            &[],
        )
        .await
        .expect("service");
        assert!(matches!(
            crate::features::services::service_push_component::push(&waiting, "my-api", Some("node.example.com")),
            Some(Push::To(target)) if target == "node.example.com/my-api/waiting:latest"
        ));

        // Created against a public image: a push to the service's own
        // repository here still lands, and this is that reference.
        let theirs = services::all(&console.database, Some(&project.id))
            .await
            .expect("query")
            .into_iter()
            .find(|service| service.slug == "web")
            .expect("the nginx one");
        // `latest`, not the `alpine` the public image carries: that
        // tag is somebody else's image's, and every service made from a
        // public base would otherwise be told to push one.
        assert!(matches!(
            crate::features::services::service_push_component::push(&theirs, "my-api", Some("node.example.com")),
            Some(Push::Elsewhere(instead))
                if instead == "node.example.com/my-api/web:latest"
        ));
    }

    /// And it is on the page, with the button that takes it.
    /// A size the ladder does not have is saved, and a database's is not.
    ///
    /// Ghost keeps its images and themes in `/var/lib/ghost/content`, and
    /// a service could not say so: only a database ever got a volume, so
    /// every deployment of it would have started from an empty site.
    #[tokio::test]
    async fn a_service_keeps_a_directory_from_its_storage_tab() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;

        let page = console
            .ui
            .with_header("cookie", cookie.clone())
            .get("/projects/shared/services/web/settings/storage")
            .await;
        let html = page.html();
        assert!(
            html.contains("action=\"/projects/shared/services/web/volumes\""),
            "{html}"
        );

        console
            .harness
            .post("/projects/shared/services/web/volumes")
            .header("cookie", &cookie)
            .form(&[("path", "/var/lib/ghost/content/"), ("name", "")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        let kept = crate::infra::repositories::volumes::of_service(&console.database, &service.id)
            .await
            .expect("volumes");
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].path, "/var/lib/ghost/content");
        assert_eq!(
            kept[0].name, "content",
            "named after the last part of the path"
        );

        let directory =
            crate::infra::repositories::volumes::ensure(&console.data_dir, "shared.web", "content")
                .expect("directory");
        std::fs::write(directory.join("image.jpg"), "bytes").expect("a file in it");
        console
            .harness
            .post(&format!(
                "/projects/shared/services/web/volumes/{}/delete",
                kept[0].id
            ))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);
        assert!(
            crate::infra::repositories::volumes::of_service(&console.database, &service.id)
                .await
                .expect("volumes")
                .is_empty()
        );
        assert!(
            directory.join("image.jpg").exists(),
            "stopping keeping it is not deleting what it holds"
        );
    }

    /// The errand that places a copy on another node carries no volumes,
    /// so that copy would start on an empty directory. Refused from both
    /// sides, before a row is written.
    #[tokio::test]
    async fn a_volume_and_a_copy_on_another_node_never_meet() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = placed_service(&console, &cookie).await;
        crate::infra::repositories::volumes::create(
            &console.database,
            &service.id,
            "content",
            "/var/lib/ghost/content",
        )
        .await
        .expect("volume");
        crate::infra::network::save(
            &console.database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-elsewhere1".into(),
                name: "alpine.example".into(),
                kind: crate::infra::network::Kind::Private,
                endpoint: None,
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("node");

        let response = console
            .harness
            .post("/projects/shared/services/web/placement")
            .header("cookie", &cookie)
            .form(&[("replicas", "1"), ("new-on", "nd-elsewhere1")])
            .send()
            .await;
        let refused = response.header("location").unwrap_or_default();
        assert!(refused.contains("volume"), "{refused}");
        let copies =
            crate::infra::repositories::replicas::of_service(&console.database, &service.id)
                .await
                .expect("copies");
        assert!(copies.iter().all(|copy| copy.is_here()));

        let kept = crate::infra::repositories::volumes::of_service(&console.database, &service.id)
            .await
            .expect("volumes");
        crate::infra::repositories::volumes::delete(&console.database, &kept[0].id)
            .await
            .expect("deleted");
        for copy in &copies {
            crate::infra::repositories::replicas::move_to(
                &console.database,
                &copy.id,
                Some("nd-elsewhere1"),
            )
            .await
            .expect("moved");
        }
        let response = console
            .harness
            .post("/projects/shared/services/web/volumes")
            .header("cookie", &cookie)
            .form(&[("path", "/data")])
            .send()
            .await;
        let refused = response.header("location").unwrap_or_default();
        assert!(refused.contains("volume"), "{refused}");
        assert!(
            crate::infra::repositories::volumes::of_service(&console.database, &service.id)
                .await
                .expect("volumes")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_databases_storage_is_not_the_operators_to_change() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        let (made, _) = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "Orders",
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&console.database)
        .await
        .expect("database");

        let response = console
            .harness
            .post(&format!("/projects/my-api/services/{}/volumes", made.slug))
            .header("cookie", &cookie)
            .form(&[("path", "/elsewhere")])
            .send()
            .await;
        assert!(response
            .header("location")
            .unwrap_or_default()
            .contains("error="));
        assert_eq!(
            crate::infra::repositories::volumes::of_service(&console.database, &made.id)
                .await
                .expect("volumes")
                .len(),
            1
        );
    }

    /// MySQL has no read-only copies, and under 256 MB it is killed
    /// however it is tuned. Both are refused at the POST, which is the
    /// check that counts: the page offering neither is a courtesy.
    #[tokio::test]
    async fn a_mysql_database_stays_one_copy_and_above_its_floor() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        let (made, _) = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "Orders",
            engine: crate::domain::entities::database::Engine::Mysql,
            version: "8.4",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&console.database)
        .await
        .expect("database");
        let base = format!("/projects/my-api/services/{}", made.slug);

        let response = console
            .harness
            .post(&format!("{base}/placement"))
            .header("cookie", &cookie)
            .form(&[("replicas", "2"), ("new-on", "")])
            .send()
            .await;
        assert!(
            response
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "a second copy was not refused"
        );
        let copies = crate::infra::repositories::replicas::of_service(&console.database, &made.id)
            .await
            .expect("query");
        assert_eq!(copies.len(), 1);

        console
            .harness
            .post(&format!("{base}/memory"))
            .header("cookie", &cookie)
            .form(&[("memory", "134217728"), ("cpu", "")])
            .send()
            .await;
        let after = services::find(&console.database, &made.id)
            .await
            .expect("query")
            .expect("still there");
        assert_eq!(after.memory_limit, Some(256 * 1024 * 1024));
    }

    /// The select was never the restriction: `services::set_memory_limit`
    /// refused anything off the ladder, so a hand-made POST was refused
    /// too. That check is a floor now — a machine truth, since below a few
    /// megabytes nothing starts — and the rung requirement stays where the
    /// reason for it lives, which is a database.
    #[tokio::test]
    async fn a_service_takes_a_size_the_ladder_does_not_have() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        let made = crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "api",
            "node.example.com/my-api/api:latest",
            &[],
        )
        .await
        .expect("service");

        console
            .harness
            .post("/projects/my-api/services/api/memory")
            .header("cookie", &cookie)
            .form(&[("memory", "300 MB"), ("cpu", "0.75")])
            .send()
            .await;

        let after = services::find(&console.database, &made.id)
            .await
            .expect("query")
            .expect("still there");
        assert_eq!(
            after.memory_limit,
            Some(300 * 1024 * 1024),
            "a size between two rungs did not survive the POST"
        );
        assert_eq!(after.cpu_millicores, Some(750));

        // And under what a container needs to start is still refused,
        // which is the whole of what the ladder was protecting.
        console
            .harness
            .post("/projects/my-api/services/api/memory")
            .header("cookie", &cookie)
            .form(&[("memory", "1 MB"), ("cpu", "")])
            .send()
            .await;
        let after = services::find(&console.database, &made.id)
            .await
            .expect("query")
            .expect("still there");
        assert_eq!(
            after.memory_limit,
            Some(300 * 1024 * 1024),
            "1 MB was accepted: {:?}",
            after.memory_limit
        );
    }

    /// A service's settings are tabs, and the set depends on the kind.
    ///
    /// Twelve sections in one column meant the way to reach the last one
    /// was to know it was there — reported by Jorge, who could not get to
    /// them by scrolling. One URL each, so a tab can be linked and read
    /// with scripting off, and the page sends only what it shows.
    ///
    /// A managed database gets three, not five: it has no image, no tag
    /// and no environment — the node writes all three from the engine's
    /// row — and no ports, edges or certificates of the kind this console
    /// serves, because Postgres speaks its own protocol on a published
    /// port. Offering it those two tabs would be offering empty pages.
    #[tokio::test]
    async fn a_tab_a_service_does_not_have_lands_on_one_it_does() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "api",
            "node.example.com/my-api/api:latest",
            &[],
        )
        .await
        .expect("service");
        let made = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "Orders",
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: crate::domain::policies::presets::LADDER[1],
        }
        .run(&console.database)
        .await
        .expect("database");

        macro_rules! landing {
            ($path:expr) => {
                console
                    .harness
                    .get(&$path)
                    .header("cookie", &cookie)
                    .send()
                    .await
                    .header("location")
                    .unwrap_or_default()
            };
        }

        // The bare path is not a page: it names the first tab each kind
        // has, which is where a link kept from before the tabs lands.
        assert_eq!(
            landing!("/projects/my-api/services/api/settings"),
            "/projects/my-api/services/api/settings/image"
        );
        let database = format!("/projects/my-api/services/{}/settings", made.0.slug);
        assert_eq!(landing!(database.clone()), format!("{database}/database"));

        // And a tab that kind does not have is a redirect, not a blank
        // page. `general` on a database, `database` on a plain service.
        assert_eq!(
            landing!(format!("{database}/general")),
            format!("{database}/database")
        );
        assert_eq!(
            landing!("/projects/my-api/services/api/settings/database"),
            "/projects/my-api/services/api/settings/image"
        );
        // As is a word that is not a tab at all.
        assert_eq!(
            landing!("/projects/my-api/services/api/settings/nonsense"),
            "/projects/my-api/services/api/settings/image"
        );
        // `Advanced` was split into three at Jorge's ask, and a link kept
        // from before names a tab that no longer exists — which is the
        // same case as a hand-typed word and lands the same way.
        assert_eq!(
            landing!("/projects/my-api/services/api/settings/advanced"),
            "/projects/my-api/services/api/settings/image"
        );
        // A database *does* have a logs tab: `try_deploy` reads
        // `log_timestamps` for every kind, so the setting was already
        // reaching Postgres and only the page withheld it.
        let logs = console
            .harness
            .get(&format!("{database}/logs"))
            .header("cookie", &cookie)
            .send()
            .await;
        assert_eq!(logs.header("location"), None, "a database has no logs tab");
        assert!(
            logs.body.contains("log-timestamps"),
            "and the form is on it: {}",
            logs.body
        );
    }

    /// The whole kept history can be searched, and a match opened in it.
    ///
    /// The last of the three things CLAUDE.md listed as open about logs:
    /// they were kept across restarts and bounded three ways, and nothing
    /// searched them. Asked for by Jorge.
    ///
    /// A GET form, so a search is a link: shareable, bookmarkable, and it
    /// works with scripting off — which is this console's rule and the
    /// reason somebody opens it at all, since they are usually here
    /// because something is wrong.
    #[tokio::test]
    async fn the_kept_history_can_be_searched_and_read_around_a_match() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        // A history in three files, as a container that restarted twice
        // leaves it. The wanted line is in the oldest, which is the case
        // the feature exists for.
        let logs = console.data_dir.join("logs");
        std::fs::create_dir_all(&logs).expect("mkdir");
        std::fs::write(
            logs.join("my-api.web.log.2"),
            "starting\nconnection refused talking to postgres\nexiting\n",
        )
        .expect("write");
        std::fs::write(logs.join("my-api.web.log.1"), "starting\nexiting\n").expect("write");
        std::fs::write(logs.join("my-api.web.log"), "starting\nlistening on 8080\n")
            .expect("write");

        let body = console
            .harness
            .get(&format!("{page}/logs?q=refused"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("connection refused talking to postgres"),
            "the match is not on the page: {body}"
        );
        // The count is of the whole log, and it says how much was read —
        // "1 of 7" and "1 of 40,000" are different answers.
        assert!(
            body.contains("7"),
            "it does not say how much it read: {body}"
        );
        // And the match links to itself in context, at the line it was
        // counted at.
        assert!(
            body.contains("at=2"),
            "no way to open the match in context: {body}"
        );

        // Which shows the lines around it, including the one after — the
        // point of context being that the error is rarely the last word.
        let around = console
            .harness
            .get(&format!("{page}/logs?q=refused&at=2"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(around.contains("starting"), "{around}");
        assert!(
            around.contains("exiting"),
            "the line after is missing: {around}"
        );

        // A needle with a `&` in it survives being put in a link, which
        // is a thing somebody searches a log for.
        let awkward = console
            .harness
            .get(&format!("{page}/logs?q=8080%20%26%20up"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            awkward.contains("Nothing matched") || awkward.contains("Nada"),
            "an awkward needle broke the page: {awkward}"
        );
    }

    /// The log budget is typed on the page and reaches the row.
    ///
    /// Retention was three constants and the per-container one was a
    /// *count* of generations, so a service that talks a lot and one that
    /// barely speaks were held to the same two files. Asked for by Jorge.
    #[tokio::test]
    async fn a_service_says_how_much_log_it_keeps() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        let made = services::in_project(&console.database, &project.id, "web")
            .await
            .expect("query")
            .expect("present");

        console
            .harness
            .post(&format!("{page}/log-timestamps"))
            .header("cookie", &cookie)
            .form(&[("log_budget", "200 MB")])
            .send()
            .await;
        let after = services::find(&console.database, &made.id)
            .await
            .expect("query")
            .expect("still there");
        assert_eq!(after.log_budget, Some(200 * 1024 * 1024));

        // Empty is the default — two rotations, whatever they weigh — and
        // not nought, which would mean keeping no history at all.
        console
            .harness
            .post(&format!("{page}/log-timestamps"))
            .header("cookie", &cookie)
            .form(&[("log_budget", "")])
            .send()
            .await;
        let after = services::find(&console.database, &made.id)
            .await
            .expect("query")
            .expect("still there");
        assert_eq!(after.log_budget, None, "empty is the default, not zero");

        // And a size below what a container needs to *start* is fine
        // here: keeping only the live log is a real answer, which is why
        // this field does not use the memory parser's floor.
        console
            .harness
            .post(&format!("{page}/log-timestamps"))
            .header("cookie", &cookie)
            .form(&[("log_budget", "1 MB")])
            .send()
            .await;
        let after = services::find(&console.database, &made.id)
            .await
            .expect("query")
            .expect("still there");
        assert_eq!(after.log_budget, Some(1024 * 1024));
    }

    /// One table: the port, what it is, where it answers, its certificate.
    ///
    /// They were two sections, so "is my service reachable and is its
    /// certificate all right" meant reading two lists and matching
    /// hostnames between them by eye. Merged at Jorge's ask.
    ///
    /// What this **cannot** cover is the source being stored as the port
    /// is created. `add_port` asks DNS whether the name points here before
    /// it accepts one, and no name resolves in a test — which is why every
    /// hostname test in this file makes its port through the platform. The
    /// storing itself is `store_source`, which the per-port form has used
    /// all along; what is new is only when it is called.
    #[tokio::test]
    async fn a_port_carries_its_certificate_in_the_same_row() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        // Without a domain the form does not offer HTTPS at all, so it
        // does not offer a source either. A first version of this asserted
        // the field and passed on the *disclosure's* copy of it — which
        // was the thing being deleted.
        named(&console, "example.com").await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("present");
        let made = services::in_project(&console.database, &project.id, "web")
            .await
            .expect("query")
            .expect("present");
        // Published *and* served over HTTPS, which is the pair a single
        // "Type" column could not hold: the two boxes are independent.
        ports::create(
            &console.database,
            &made.id,
            8080,
            true,
            Some("api.example.com"),
            &[],
        )
        .await
        .expect("port");
        crate::infra::edge::policy::set(
            &console.database,
            "api.example.com",
            &crate::infra::edge::policy::RenewWith::SelfSigned,
        )
        .await
        .expect("policy");

        let body = console
            .harness
            .get(&format!("{page}/settings/network"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(body.contains("HTTPS · TCP"), "both answers: {body}");
        assert!(body.contains("api.example.com"), "{body}");
        // The certificate is in the row rather than in a section below.
        assert!(
            !body.contains(r#"id="certificates""#),
            "the old section is still there: {body}"
        );
        assert!(body.contains("self-signed"), "no source in the row: {body}");
        // And the form asks for one on the way in, revealed with the HTTPS
        // box — a port created with HTTPS used to take the default and
        // start ordering from Let's Encrypt before anybody could say
        // otherwise, against an account that locks at five failures an
        // hour.
        assert!(
            body.contains(r#"name="renew_with""#),
            "the add form does not offer a source: {body}"
        );
    }

    /// The other row from Jorge's node: an image from Docker Hub.
    ///
    /// `Coffe Landing` in `Coffe Store`, running
    /// `docker.io/library/nginx:alpine`. It had a paragraph explaining
    /// that a push could not reach it; Jorge said that made no sense, it
    /// went, and the card was then **empty** on the one service he was
    /// looking at — which is how this got found.
    ///
    /// The paragraph was wrong as well as unreadable. The registry matches
    /// a push by `<project>/<service>` as well as by the declared image,
    /// so the reference this offers is a command that works today and
    /// adopts the pushed image. Saying "you cannot" was the console
    /// describing a rule it had already stopped having.
    #[tokio::test]
    async fn a_service_running_somebody_elses_image_still_has_a_command() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        crate::infra::node::settings::set_domain(&console.database, Some("node-1.tobaw.shop"))
            .await
            .expect("domain");
        let project =
            crate::infra::repositories::projects::create(&console.database, "Coffe Store")
                .await
                .expect("project");
        crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "Coffe Landing",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");

        let body = console
            .harness
            .get("/projects/coffe-store/services/coffe-landing/settings/image")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("docker push node-1.tobaw.shop/coffe-store/coffe-landing:latest"),
            "no command for a service running an image from elsewhere: {body}"
        );
        // The prerequisite is a link to where a token is made, not a
        // sentence naming a page and leaving somebody to find it.
        assert!(
            body.contains(r#"href="/projects/coffe-store/settings""#),
            "no way to the tokens: {body}"
        );
        // And it says what pushing it does, in one line rather than four.
        assert!(body.contains("replaces the image"), "{body}");
    }

    /// The row from Jorge's node, rendered here.
    ///
    /// He reported not seeing the command at all, and the binary on the
    /// node was mine by sha256 — so the question was whether his *data*
    /// produced it, not whether the code shipped. This is that row:
    /// pushed under a tag that is not `latest`, watching every tag.
    #[tokio::test]
    async fn the_command_appears_for_a_service_pushed_under_another_tag() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        crate::infra::node::settings::set_domain(&console.database, Some("node-1.tobaw.shop"))
            .await
            .expect("domain");
        let project = crate::infra::repositories::projects::create(&console.database, "wabot")
            .await
            .expect("project");
        let made = crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "nginx-2",
            "node-1.tobaw.shop/wabot/nginx-2:new",
            &[],
        )
        .await
        .expect("service");
        services::set_tracking(&console.database, &made.id, Some("*"), true)
            .await
            .expect("watch everything");

        let body = console
            .harness
            .get("/projects/wabot/services/nginx-2/settings/image")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("docker push node-1.tobaw.shop/wabot/nginx-2:latest"),
            "no command for the row this was reported against: {body}"
        );
    }

    /// The image tab: what it runs, then what a push does to it.
    ///
    /// Ordered by Jorge, section by section — the image first, because a
    /// tab that opened on a rule about pushes said what happens to the
    /// service before saying what the service is; then the switch, then
    /// the tag it governs, then the one command to type.
    #[tokio::test]
    async fn the_image_tab_reads_in_the_order_it_was_asked_for() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        crate::infra::node::settings::set_domain(&console.database, Some("node.example.com"))
            .await
            .expect("domain");
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        let made = crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "api",
            "node.example.com/my-api/api:latest",
            &[],
        )
        .await
        .expect("service");
        services::set_tracking(
            &console.database,
            &made.id,
            Some(crate::domain::entities::service::EVERY_TAG),
            true,
        )
        .await
        .expect("watch everything");

        let body = console
            .harness
            .get("/projects/my-api/services/api/settings/image")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        let at = |needle: &str| {
            body.find(needle)
                .unwrap_or_else(|| panic!("{needle} is not on the tab: {body}"))
        };
        // The image, then the switch, then the tag, then the command.
        assert!(
            at("Save image") < at("auto_deploy"),
            "the image comes first"
        );
        assert!(
            at("auto_deploy") < at(r#"id="track_tag""#),
            "then the switch"
        );
        assert!(
            at(r#"id="track_tag""#) < at("docker push"),
            "then the command"
        );

        // **A star is a rule, not a tag.** This service watches every tag,
        // and the example still has to be something somebody can paste:
        // `docker push …:*` is a command that fails.
        assert!(
            body.contains("docker push node.example.com/my-api/api:latest"),
            "the example is not pastable: {body}"
        );
        assert!(!body.contains(":*"), "a star reached the command: {body}");
        // And it can be taken.
        assert!(body.contains(r#"data-island="copy""#), "{body}");
    }

    /// A service takes any size; a database picks from the list.
    ///
    /// Jorge asked for the free choice and named the exception in the same
    /// breath, and the exception is the interesting half: a database's
    /// ceiling is not only a cgroup limit, it picks `shared_buffers` and
    /// five other numbers through `postgres::tuning`. A rung is a size an
    /// operator can reason about *as a configuration*; 300 MB is not.
    ///
    /// The form is asserted here and the POST in
    /// `a_service_takes_a_size_the_ladder_does_not_have`, because a select
    /// is a courtesy the browser offers and the check that counts is on
    /// the way in.
    #[tokio::test]
    async fn only_a_database_chooses_its_memory_from_a_list() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "api",
            "node.example.com/my-api/api:latest",
            &[],
        )
        .await
        .expect("service");

        let plain = console
            .harness
            .get("/projects/my-api/services/api/settings/resources")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        // The element name is assembled rather than written, because
        // `every_field_says_what_may_be_autofilled_into_it` reads this
        // file as text and takes the opening of a field literal for a
        // field — one with no `autocomplete` on it. It found these three,
        // and then found the sentence that explained the first fix.
        let input = |id: &str| format!("{}{} id=\"{id}\"", "<inp", "ut");
        assert!(
            plain.contains(&input("memory-size")),
            "a plain service should take any size: {plain}"
        );
        assert!(
            plain.contains(&input("cpu-size")),
            "and any CPU ceiling: {plain}"
        );
        // The rungs are still there — as suggestions, which is what a list
        // is good for once it stops being the only answer.
        assert!(
            plain.contains(r#"<datalist id="memory-sizes""#),
            "the ladder is gone rather than offered: {plain}"
        );

        let made = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "Orders",
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: crate::domain::policies::presets::LADDER[1],
        }
        .run(&console.database)
        .await
        .expect("database");
        let body = console
            .harness
            .get(&format!(
                "/projects/my-api/services/{}/settings/resources",
                made.0.slug
            ))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains(r#"<select id="memory-size""#),
            "a database's memory is a list: {body}"
        );
        // And its CPU is not: nothing derives a setting from it.
        assert!(
            body.contains(&input("cpu-size")),
            "a database's CPU should be free: {body}"
        );
    }

    /// The digest sits in a block, so it renders under the tag rather
    /// than against it.
    ///
    /// They read as `newsha256:af618a130cbe`, one unreadable word, under a
    /// comment promising two lines. `.tile-detail` on a `span`: the class
    /// carries colour and a zeroed margin and says nothing about
    /// `display`, so it styled the text and left the layout at inline.
    ///
    /// The assertion is on the element rather than the text because the
    /// text never showed it — the first version of this test passed
    /// against the broken markup, since `new<span …>sha256:` does not
    /// contain `newsha256:`. Nothing separated them on screen and
    /// everything separated them in the source, which is why only Jorge
    /// could see it.
    #[tokio::test]
    async fn a_release_shows_its_digest_under_its_tag() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        let made = crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "api",
            "node.example.com/my-api/api:latest",
            &[],
        )
        .await
        .expect("service");
        crate::infra::repositories::releases::record(
            &console.database,
            &made.id,
            "node.example.com/my-api/api:new",
            "sha256:af618a130cbe1ecf7a82570bc32056e00e4c01edba81aca520bc1890fdee5009",
            crate::domain::entities::release::Source::Push,
        )
        .await
        .expect("release");

        let body = console
            .harness
            .get("/projects/my-api/services/api")
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        // The short digest as *text*, not the full one inside the cell's
        // `title` — which is where a plain `find` lands, and did.
        let rendered = "sha256:af618a130cbe<";
        let at = body
            .find(rendered)
            .unwrap_or_else(|| panic!("no digest on the page: {body}"));
        let opens = body[..at].rfind('<').expect("an element around it");
        let element = &body[opens..at];
        assert!(
            element.starts_with("<div"),
            "the digest is inline against the tag, not on its own line: {element}"
        );
    }

    #[tokio::test]
    async fn the_releases_card_says_how_to_push() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        crate::infra::node::settings::set_domain(&console.database, Some("node.example.com"))
            .await
            .expect("domain");
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        crate::infra::repositories::services::create(
            &console.database,
            &project.id,
            "api",
            "node.example.com/my-api/api:latest",
            &[],
        )
        .await
        .expect("service");

        let body = console
            .harness
            .get("/projects/my-api/services/api")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("docker push node.example.com/my-api/api:latest"),
            "{body}"
        );
        assert!(
            body.contains(r#"data-island="copy""#),
            "and something to copy it with: {body}"
        );

        // And a service on a public image says a push here lands anyway.
        // **It said the opposite** for seven weeks after the registry
        // began matching a service's own repository by slug — found on a
        // VM node, where the push it called impossible deployed.
        let body = console
            .harness
            .get("/projects/my-api/services/web")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("an image from another registry"),
            "it says what it runs: {body}"
        );
        assert!(
            body.contains("lands on it all the same"),
            "and that a push here still lands: {body}"
        );
        assert!(body.contains("watching :latest"), "{body}");
        assert!(
            body.contains("docker push node.example.com/my-api/web:latest"),
            "and the commands for what would work: {body}"
        );
    }

    async fn service(console: &Console, cookie: &str) -> String {
        let slug = project(console, cookie).await;
        console
            .harness
            .post(&format!("/projects/{slug}/services"))
            .header("cookie", cookie)
            .form(&[("name", "web"), ("image", "docker.io/library/nginx:alpine")])
            .send()
            .await;
        "/projects/my-api/services/web".to_string()
    }

    /// The split: one page says what the service is doing, the other
    /// changes it. A form that drifts back onto the service page puts
    /// a text box next to the state somebody is watching, which is the
    /// crowding this separation exists to undo.
    #[tokio::test]
    async fn the_service_page_reads_and_the_settings_page_writes() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let reading = console
            .harness
            .get(&page)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        for form in [
            "/tracking\"",
            "/env\"",
            "name=\"container_port\"",
            "name=\"env\"",
        ] {
            assert!(
                !reading.contains(form),
                "{form} is still on the service page: {reading}"
            );
        }
        assert!(reading.contains("/settings"), "and it links to them");

        // And each form is on exactly one tab. Naming the tab is the point
        // of the assertion: the four used to be one column somebody
        // scrolled, so "it is on the settings page" was all there was to
        // say. Now where a thing lives is a fact worth pinning, and a
        // section that quietly appears on two tabs is a section somebody
        // saves twice.
        let mut seen = Vec::new();
        for tab in [
            "image",
            "environment",
            "resources",
            "network",
            "replicas",
            "logs",
            "danger",
        ] {
            let body = console
                .harness
                .get(&format!("{page}/settings/{tab}"))
                .header("cookie", &cookie)
                .send()
                .await
                .body;
            for form in [
                "/tracking\"",
                "/env\"",
                "name=\"container_port\"",
                "name=\"env\"",
            ] {
                if body.contains(form) {
                    seen.push((form, tab));
                }
            }
            // The bar is on every one of them, and says which is current.
            assert!(
                body.contains(r#"aria-current="page""#),
                "the {tab} tab does not say it is current: {body}"
            );
        }
        assert_eq!(
            seen,
            vec![
                ("/tracking\"", "image"),
                ("/env\"", "environment"),
                ("name=\"env\"", "environment"),
                ("name=\"container_port\"", "network"),
            ],
            "a form is missing, or on more than one tab"
        );
    }

    /// A database's storage was a number in a column and nothing that
    /// said what it *is*: a directory per copy, on the machine running
    /// it, that outlives every deployment. Reported by Jorge, who went
    /// looking for the persistent disks and found none named anywhere.
    ///
    /// Only where there is one. A service that declares no volume keeps
    /// nothing, and the card would be a row of dashes explaining an
    /// absence nobody asked about.
    #[tokio::test]
    async fn a_database_names_the_disk_each_copy_keeps() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let plain = console
            .harness
            .get(&page)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            !plain.contains("Persistent disks"),
            "a service that keeps nothing says nothing: {plain}"
        );

        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        let (database, _) = crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&console.database)
        .await
        .expect("database");
        crate::infra::repositories::replicas::ensure_here(&console.database, &database.id, 2)
            .await
            .expect("a second copy");

        let body = console
            .harness
            .get("/projects/my-api/services/orders")
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Persistent disks"), "{body}");
        // One row per copy, and the mount inside the container is what
        // names it — the path on the node is derived from the container
        // id and is not somebody's to type.
        assert!(
            body.matches(crate::domain::policies::postgres::DATA_MOUNT)
                .count()
                >= 2,
            "a disk for each copy: {body}"
        );
    }

    /// A hostname that names another node tells that node.
    ///
    /// **Found on the real network.** `ports::create` writes the edge row
    /// for the nodes a hostname resolves to, which since the DNS check
    /// can be a machine that is not this one — and nothing told it. The
    /// row said the public node answered for `nginx.node-1.tobaw.shop`,
    /// it was written one millisecond after the port, and the public node
    /// had never heard of the name: stored, shown as served, and served by
    /// nobody, which is the fault that row exists to prevent, in the one
    /// direction it did not cover.
    #[tokio::test]
    async fn a_port_that_names_another_node_sends_it_an_errand() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
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
        .expect("the other node");

        console
            .harness
            .post("/projects/my-api/services/web/ports")
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "8080"),
                ("https", "1"),
                ("hostname", "localhost"),
                ("renew_with", "self_signed"),
            ])
            .send()
            .await;

        let orders = crate::infra::network::errand::all(&console.database)
            .await
            .expect("errands");
        let told = orders
            .iter()
            .filter(|order| order.node_id == "nd-theedge")
            .filter_map(|order| {
                serde_json::from_value::<crate::infra::network::errand::Edge>(order.payload.clone())
                    .ok()
            })
            .next_back()
            .expect("the node the name points at was told");
        assert_eq!(told.hostname, "localhost");

        // And saying it again says nothing: the instruction is compared,
        // like a placement, so a deployment that changes nothing does not
        // queue a second errand for the same name.
        let before = orders.len();
        console
            .harness
            .post("/projects/my-api/services/web/edges")
            .header("cookie", &cookie)
            .form(&[("hostname", "localhost"), ("edge-nd-theedge", "1")])
            .send()
            .await;
        assert_eq!(
            crate::infra::network::errand::all(&console.database)
                .await
                .expect("errands")
                .len(),
            before,
            "the same instruction twice is one errand"
        );
    }

    /// A service whose port answers at `localhost`, served by a node of
    /// its own that has been told so — the state the next two tests take
    /// apart.
    async fn served_elsewhere(console: &Console, cookie: &str) {
        service(console, cookie).await;
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
        .expect("the other node");
        console
            .harness
            .post("/projects/my-api/services/web/ports")
            .header("cookie", cookie)
            .form(&[
                ("container_port", "8080"),
                ("https", "1"),
                ("hostname", "localhost"),
                ("renew_with", "self_signed"),
            ])
            .send()
            .await;
        // What a deployment sends once a copy runs. Nothing runs here, so
        // the errand the port queued has nothing behind it — which is
        // already a withdrawal, and would make these tests prove nothing.
        let serving = serde_json::to_value(crate::infra::network::errand::Edge {
            hostname: "localhost".into(),
            upstreams: vec!["10.42.0.3:30000".into()],
            peers: Vec::new(),
            secret: None,
            redirect_to: None,
        })
        .expect("payload");
        crate::infra::network::errand::queue(
            &console.database,
            "nd-theedge",
            crate::infra::network::errand::Kind::Edge,
            &serving,
        )
        .await
        .expect("served");
        assert!(
            crate::infra::network::errand::names_told(&console.database)
                .await
                .expect("told")
                .contains(&("nd-theedge".to_string(), "localhost".to_string())),
            "the other node was never told to answer for the name"
        );
    }

    /// **Deleting a port left the other nodes answering for its name**,
    /// with a certificate renewed for it twice a day: the rows that would
    /// have told them were the ones the delete removed. Found while adding
    /// redirects, 2026-10-20.
    #[tokio::test]
    async fn deleting_a_port_tells_the_nodes_that_answered_for_its_name_to_stop() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        served_elsewhere(&console, &cookie).await;

        let service = crate::infra::repositories::services::all(&console.database, None)
            .await
            .expect("services")
            .pop()
            .expect("web");
        let port = crate::infra::repositories::ports::of_service(&console.database, &service.id)
            .await
            .expect("ports")
            .pop()
            .expect("the named port");
        console
            .harness
            .post(&format!(
                "/projects/my-api/services/web/ports/{}/delete",
                port.id
            ))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(
            crate::infra::network::errand::names_told(&console.database)
                .await
                .expect("told")
                .is_empty(),
            "the other node still answers for the name"
        );
    }

    #[tokio::test]
    async fn deleting_a_service_tells_the_nodes_that_answered_for_its_names_to_stop() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        served_elsewhere(&console, &cookie).await;

        console
            .harness
            .post("/projects/my-api/services/web/delete")
            .header("cookie", &cookie)
            .form(&[("confirm", "web")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(
            crate::infra::network::errand::names_told(&console.database)
                .await
                .expect("told")
                .is_empty(),
            "the other node still answers for the name"
        );
    }

    /// Two nodes for one name, and each is told who the other is.
    ///
    /// **Because two edges of one name may never have met.** The overlay
    /// is a star: nodes enrolled by the same authority have no session and
    /// no rows about each other. The node that owns the service is the
    /// only one that knows the whole set, so the set travels on the errand
    /// that makes each of them an edge — with an address, a key and the
    /// authority that vouches for its certificate, which is what
    /// `call::to_node` needs to verify rather than merely encrypt.
    ///
    /// What it is all for is the HTTP-01 challenge: the token lives in the
    /// ordering node's own table, and the authority asks whichever address
    /// the name resolves to.
    #[tokio::test]
    async fn each_edge_of_a_name_is_told_who_the_others_are() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        service(&console, &cookie).await;
        for (id, name) in [
            ("nd-first1", "first.example"),
            ("nd-second1", "second.example"),
        ] {
            crate::infra::network::save(
                &console.database,
                &crate::infra::network::Node {
                    update_note: None,
                    version: None,
                    wanted_version: None,
                    usage: None,
                    draining_at: None,
                    capacity: None,
                    id: id.into(),
                    name: name.into(),
                    kind: crate::infra::network::Kind::Public,
                    // `localhost` for both: it is the one name that
                    // resolves the same everywhere, so it stands in for
                    // two machines the hostname points at.
                    endpoint: Some("localhost:443".into()),
                    public_key: Some("0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into()),
                    overlay_ip: Some(format!("10.42.0.{}", id.len())),
                    is_self: false,
                    last_seen_at: None,
                    allows: vec![crate::domain::values::capability::Capability::Edge],
                    ca_pem: Some(
                        "-----BEGIN CERTIFICATE-----\nnot real\n-----END CERTIFICATE-----\n".into(),
                    ),
                },
            )
            .await
            .expect("a node");
        }

        console
            .harness
            .post("/projects/my-api/services/web/ports")
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "8080"),
                ("https", "1"),
                ("hostname", "localhost"),
                ("renew_with", "self_signed"),
            ])
            .send()
            .await;
        console
            .harness
            .post("/projects/my-api/services/web/edges")
            .header("cookie", &cookie)
            .form(&[
                ("hostname", "localhost"),
                ("edge-nd-first1", "1"),
                ("edge-nd-second1", "1"),
            ])
            .send()
            .await;

        let orders = crate::infra::network::errand::all(&console.database)
            .await
            .expect("errands");
        let to_first = orders
            .iter()
            .filter(|order| order.node_id == "nd-first1")
            .filter_map(|order| {
                serde_json::from_value::<crate::infra::network::errand::Edge>(order.payload.clone())
                    .ok()
            })
            .next_back()
            .expect("an edge errand for the first node");

        assert_eq!(
            to_first
                .peers
                .iter()
                .map(|peer| peer.node_id.clone())
                .collect::<Vec<_>>(),
            vec!["nd-second1".to_string()],
            "the other edge, and not itself"
        );
        let peer = &to_first.peers[0];
        assert_eq!(peer.overlay_ip, "10.42.0.10");
        assert!(
            !peer.public_key.is_empty(),
            "an address alone is no session"
        );
        assert!(
            peer.ca_pem.contains("BEGIN CERTIFICATE"),
            "and verified, not merely encrypted"
        );
        assert!(
            to_first.secret.is_some(),
            "and something to prove itself with when it publishes a challenge"
        );
        // The same secret for both, because it belongs to the name.
        let to_second = orders
            .iter()
            .filter(|order| order.node_id == "nd-second1")
            .filter_map(|order| {
                serde_json::from_value::<crate::infra::network::errand::Edge>(order.payload.clone())
                    .ok()
            })
            .next_back()
            .expect("an edge errand for the second node");
        assert_eq!(to_first.secret, to_second.secret);
    }

    /// A name pointing at another node is accepted, and that node is the
    /// one recorded as answering for it.
    ///
    /// Reported by Jorge: exposing a service through a node that is on
    /// the network *and can be an edge* was refused, because the check
    /// asked "does this name point **here**". A service served from
    /// somewhere else is what `service_edge` has meant since phase 7, and
    /// the form was the one place that had not heard.
    ///
    /// `localhost` stands in for the other node's address: it is the one
    /// name that resolves the same everywhere, so it is the only real
    /// lookup a test can make without a network.
    #[tokio::test]
    async fn a_hostname_pointing_at_another_edge_node_is_accepted() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let service = service(&console, &cookie).await;
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
                // Public with an endpoint, and it allows this node to ask
                // — both halves, or it is not somewhere a name can be
                // sent. The endpoint is a *name*, which is what makes the
                // lookup possible at all.
                kind: crate::infra::network::Kind::Public,
                endpoint: Some("localhost:443".into()),
                public_key: Some("k".into()),
                overlay_ip: Some("10.42.0.9".into()),
                is_self: false,
                last_seen_at: None,
                allows: vec![crate::domain::values::capability::Capability::Edge],
                ca_pem: None,
            },
        )
        .await
        .expect("the other node");

        let response = console
            .harness
            .post("/projects/my-api/services/web/ports")
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "8080"),
                ("https", "1"),
                ("hostname", "localhost"),
                ("renew_with", "self_signed"),
            ])
            .send()
            .await;
        let location = response.header("location").unwrap_or_default().to_string();
        assert!(
            !location.contains("error="),
            "a name on another edge node is a name this node may store: {location}"
        );

        // And the row says *that* node answers for it. The old default —
        // always this one — would have stored a hostname shown as served
        // and served by nobody.
        let serving = crate::infra::repositories::edges::nodes_for(&console.database, "localhost")
            .await
            .expect("query");
        assert!(
            serving.contains(&"nd-theedge".to_string()),
            "the node the name points at is recorded: {serving:?}"
        );
        let _ = service;

        // A name that points nowhere is still refused, and the message
        // names every node that could have served it — an operator
        // cannot pick from a list they were not shown.
        let refused = console
            .harness
            .post("/projects/my-api/services/web/ports")
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "8081"),
                ("https", "1"),
                ("hostname", "nothing.invalid"),
                ("renew_with", "self_signed"),
            ])
            .send()
            .await;
        assert!(
            refused
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "{:?}",
            refused.header("location")
        );
    }

    /// Each form comes back to the page it is on. Landing on the
    /// service page after saving means the next edit starts with a
    /// click somebody should not have had to make.
    ///
    /// **Every form on the page, and that is the point of the list.**
    /// This guarded three of them and each one added since went
    /// somewhere else: placement and edges to the detail page, because
    /// that is where those cards used to live, and a database's name,
    /// certificate and port to the project's list, because the locator
    /// they share answers with the project. Reported by Jorge, who saved
    /// a setting and found himself two pages away.
    /// Who answers for a name is chosen only for a name the service has.
    /// The row is what the renewal loop orders for, so one written from a
    /// form field for any name at all was a certificate asked for
    /// something nothing serves. Found on a VM node.
    #[tokio::test]
    async fn an_edge_is_chosen_only_for_a_name_the_service_has() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("{page}/edges"))
            .header("cookie", &cookie)
            .form(&[("hostname", "not-ours.example.com")])
            .send()
            .await;
        assert!(
            response
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "{:?}",
            response.header("location")
        );
        assert!(crate::infra::repositories::edges::nodes_for(
            &console.database,
            "not-ours.example.com"
        )
        .await
        .expect("query")
        .is_empty());
    }

    #[tokio::test]
    async fn saving_a_setting_stays_on_the_settings_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        let settings = format!("{page}/settings");

        // And it comes back to the *card*, not to the top of the page:
        // a form submit is a navigation, so the browser puts the reader
        // at the top of a page they were half-way down. The fragment is
        // the only lever there is with scripting off, which is the
        // console's rule — the runtime boosts links and not submits.
        //
        // Since the sections became tabs, the page each form returns to is
        // one of several — so the anchor is looked for on **the page the
        // redirect actually names**, fetched from the `Location` itself
        // rather than from a URL this test decides. A form that came back
        // to the wrong tab would otherwise pass by landing on a page whose
        // anchor happens to exist somewhere else.

        // The edges card chooses who answers for a name the service
        // already has, and a name it does not have is refused.
        let named = services::all(&console.database, None)
            .await
            .expect("query")
            .into_iter()
            .find(|row| page.ends_with(&format!("/services/{}", row.slug)))
            .expect("the service");
        crate::infra::repositories::ports::create(
            &console.database,
            &named.id,
            9090,
            false,
            Some("api.example.com"),
            &[],
        )
        .await
        .expect("a port with the name");

        // `on_page` is false for a card that renders only sometimes:
        // the edges one needs a public node, and none exists here. Its
        // redirect is still pinned — it is the id that cannot be looked
        // for.
        for (action, anchor, on_page, form) in [
            ("tracking", "releases", true, vec![("track_tag", "latest")]),
            ("env", "environment", true, vec![("env", "A=1")]),
            ("ports", "ports", true, vec![("container_port", "8080")]),
            ("placement", "placement", true, vec![("copies", "1")]),
            (
                "edges",
                "edges",
                false,
                vec![("hostname", "api.example.com")],
            ),
        ] {
            let response = console
                .harness
                .post(&format!("{page}/{action}"))
                .header("cookie", &cookie)
                .form(&form)
                .send()
                .await;
            let location = response.header("location").expect("redirected");
            assert!(
                location.starts_with(&settings),
                "{action} went to {location}"
            );
            assert!(
                location.ends_with(&format!("#{anchor}")),
                "{action} landed at the top: {location}"
            );
            // The anchor has to name something. An id nothing carries
            // is a fragment the browser ignores, silently, which is the
            // half-applied shape this console has been bitten by
            // before.
            let landed = console
                .harness
                .get(location.split('#').next().expect("a path"))
                .header("cookie", &cookie)
                .send()
                .await
                .body;
            assert!(
                !on_page || landed.contains(&format!("id=\"{anchor}\"")),
                "nothing on the page {action} lands on is #{anchor}"
            );
        }

        // And a database's three, which are the same page and a
        // different controller — against a real one, so a save that
        // works is what is being followed rather than a refusal that
        // happens to land in the same place.
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("made");
        crate::features::databases::create_database_command::CreateDatabase {
            project_id: &project.id,
            name: "orders",
            engine: crate::domain::entities::database::Engine::Postgres,
            version: "17",
            memory_limit: 256 * 1024 * 1024,
        }
        .run(&console.database)
        .await
        .expect("database");
        let database = "/projects/my-api/services/orders";
        // A certificate is refused outright on a node with no name of
        // its own, and a refusal returns to the top of the page — which
        // is the behaviour above, not the one under test here.
        crate::infra::node::settings::set_domain(&console.database, Some("node.example.com"))
            .await
            .expect("domain");

        // `saved` is false where a test cannot get a save through: a
        // certificate of any source is refused unless the node's own
        // domain resolves, and no name resolves in here. The page is
        // still pinned — that is what was wrong — and the fragment is
        // followed by the two beside it.
        for (action, anchor, saved, form) in [
            // The size is a database's card. A plain service has a
            // ceiling too and no form for it — the one place it is
            // chosen is here.
            ("memory", "memory", true, vec![("memory", "")]),
            (
                "database/name",
                "name",
                true,
                vec![("name", "orders.example.com")],
            ),
            (
                "database/certificate",
                "certificate",
                false,
                vec![("renew_with", "self-signed")],
            ),
            (
                "database/publish",
                "published",
                true,
                vec![("publish", "1")],
            ),
        ] {
            let response = console
                .harness
                .post(&format!("{database}/{action}"))
                .header("cookie", &cookie)
                .form(&form)
                .send()
                .await;
            let location = response.header("location").expect("redirected");
            assert!(
                location.starts_with(&format!("{database}/settings")),
                "{action} went to {location}"
            );
            assert!(
                !saved || location.ends_with(&format!("#{anchor}")),
                "{action} landed at the top: {location}"
            );
            let landed = console
                .harness
                .get(location.split('#').next().expect("a path"))
                .header("cookie", &cookie)
                .send()
                .await
                .body;
            assert!(
                landed.contains(&format!("id=\"{anchor}\"")),
                "nothing on the page {action} lands on is #{anchor}"
            );
        }
    }

    /// A viewer may read the service and change nothing, so the page
    /// made of nothing but controls is not a page for them.
    #[tokio::test]
    async fn a_viewer_asking_for_settings_gets_the_service() {
        let console = Console::new().await;
        let admin = console.signed_in().await;
        let page = service(&console, &admin).await;
        let member = console.joined_as(&admin, "watcher").await;

        let watcher = crate::infra::accounts::all(&console.database)
            .await
            .expect("query")
            .into_iter()
            .find(|account| account.username == "watcher")
            .expect("joined");
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("present");
        access::grant(
            &console.database,
            &watcher.id,
            &project.id,
            crate::domain::values::roles::ProjectRole::Viewer,
        )
        .await
        .expect("granted");

        let response = console
            .harness
            .get(&format!("{page}/settings/network"))
            .header("cookie", &member)
            .send()
            .await;
        assert_eq!(
            response.header("location"),
            Some(page.as_str()),
            "sent to the page they may read"
        );
    }

    /// A hostname declared through `ports::create` rather than the
    /// form: the form checks DNS first, and no name resolves to a test.
    /// Give the node a domain the way the console does: the setting, and
    /// the node's own row that every "who answers for a name" reads.
    async fn named(console: &Console, domain: &str) {
        crate::infra::node::settings::set_domain(&console.database, Some(domain))
            .await
            .expect("domain");
        crate::infra::network::ensure_self(&console.database, &console.config)
            .await
            .expect("the row follows");
    }

    async fn hostname_on(console: &Console, hostname: &str) -> String {
        let project = crate::infra::repositories::projects::find(&console.database, "my-api")
            .await
            .expect("query")
            .expect("present");
        let service = services::in_project(&console.database, &project.id, "web")
            .await
            .expect("query")
            .expect("present");
        let port = ports::create(
            &console.database,
            &service.id,
            80,
            false,
            Some(hostname),
            &[],
        )
        .await
        .expect("port");
        port.id
    }

    /// The certificate is chosen when the port is exposed, and not after.
    ///
    /// Every name the node serves gets the same three answers — the node's
    /// own domain had a page first, and that is the only thing special
    /// about it. What changed is *where* the choice is made: this used to
    /// be a control per hostname, one row of the old certificates section,
    /// and Jorge asked for it to go. Changing it means removing the port
    /// and exposing it again, which is one way to do a thing rather than
    /// two, and it puts the choice before the first order rather than
    /// after it.
    #[tokio::test]
    async fn the_certificate_is_chosen_when_the_port_is_exposed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        named(&console, "example.com").await;
        let port_id = hostname_on(&console, "api.example.com").await;

        let body = console
            .harness
            .get(&format!("{page}/settings/network"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("api.example.com"), "{body}");
        // All three, on the form that exposes a port.
        assert!(
            body.contains("Read from files on this node"),
            "the form does not offer all three: {body}"
        );
        // And nothing per hostname afterwards.
        assert!(
            !body.contains(&format!("{page}/ports/{port_id}/certificate")),
            "a per-port control is still rendered: {body}"
        );
    }

    /// The hostname field is the case that could not be done in HTML
    /// alone: `required` only when the HTTPS box is ticked. The field
    /// is still in the markup and the server still refuses an empty one
    /// — the attribute is a courtesy, not the check.
    #[tokio::test]
    async fn the_hostname_field_is_conditioned_on_the_https_box() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        hostname_on(&console, "api.example.com").await;
        // Without a domain the node offers no hostname field at all —
        // it cannot check that a name points here, so there is nothing
        // to type. That branch is a different test.
        named(&console, "node.example.com").await;

        let body = console
            .harness
            .get(&format!("{page}/settings/network"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(
            body.contains("data-required-when=\"https\""),
            "required only with the box: {body}"
        );
        crate::web::console::tests::conditions_name_real_controls(&body);
    }

    /// The default answer: a service exposes nothing.
    ///
    /// Both pages say it, and they say different halves of it — the
    /// service card on the detail page lists what there is to connect
    /// to, and the ports section in settings is where one is added.
    #[tokio::test]
    async fn a_new_service_exposes_nothing() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let body = console
            .harness
            .get(&page)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("It answers inside the project only"),
            "{body}"
        );

        let body = console
            .harness
            .get(&format!("{page}/settings/network"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("exposes nothing"), "{body}");
    }

    #[tokio::test]
    async fn a_plain_port_is_reachable_from_the_project_only() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "8080")])
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        let stored = ports::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("one port");
        assert_eq!(stored.container_port, 8080);
        assert_eq!(stored.host_port, None, "not published");
        assert_eq!(stored.hostname, None, "no hostname");

        let body = console
            .harness
            .get(&page)
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("the project only"), "{body}");
    }

    #[tokio::test]
    async fn publishing_gives_the_port_a_node_port() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "5432"), ("publish", "1")])
            .send()
            .await;

        let stored = ports::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("one port");
        assert!(stored.host_port.is_some(), "a node port was allocated");
    }

    /// The check that has to happen before the row is written. A
    /// hostname pointing somewhere else is a certificate request that
    /// fails and a page that never loads.
    #[tokio::test]
    async fn a_hostname_that_does_not_resolve_here_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let response = console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "80"),
                ("https", "1"),
                // Reserved by RFC 2606 so it can never resolve.
                ("hostname", "nothing.invalid"),
            ])
            .send()
            .await;

        response.assert_status(StatusCode::SEE_OTHER);
        let location = response.header("location").expect("redirected");
        assert!(location.contains("error="), "{location}");
        assert!(
            ports::all(&console.database)
                .await
                .expect("query")
                .is_empty(),
            "nothing was written"
        );
    }

    /// A hostname without a certificate yet has to say so: the route
    /// exists the moment the port does, and the certificate follows a
    /// few seconds later. A page that claimed it was ready would be
    /// wrong for exactly the window somebody is watching.
    ///
    /// In settings, beside the name — that is where the name was typed,
    /// and where the ports table lives now that the detail page carries
    /// the connection strings rather than a table of them.
    #[tokio::test]
    async fn a_hostname_says_when_its_certificate_has_not_arrived() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        // Written straight in: the DNS check would refuse a name that
        // does not resolve, and this test is about what the page says
        // afterwards.
        let stored = services::all(&console.database, None)
            .await
            .expect("query")
            .pop()
            .expect("one service");
        ports::create(
            &console.database,
            &stored.id,
            80,
            false,
            Some("api.example.com"),
            &[],
        )
        .await
        .expect("port");

        let body = console
            .harness
            .get(&format!("{page}/settings/network"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("Certificate on the way"), "{body}");
    }

    /// A node with no domain of its own cannot check that a name points
    /// at it, so `add_port` refuses every hostname on that ground. The
    /// form offered the box and a text field anyway, under a hint that
    /// said to "type a hostname already pointed here" — which could
    /// only ever come back as an error.
    #[tokio::test]
    async fn https_is_not_offered_until_the_node_has_a_domain() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        let body = console
            .harness
            .get(&format!("{page}/settings/network"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("needs a domain"), "it says why: {body}");
        assert!(
            !body.contains("api.example.com"),
            "and offers no field to fill in: {body}"
        );

        // The disabled box is a courtesy; this is the check that counts.
        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "80"),
                ("https", "1"),
                ("hostname", "api.example.com"),
            ])
            .send()
            .await;
        assert!(
            ports::all(&console.database)
                .await
                .expect("query")
                .is_empty(),
            "a hostname was accepted with no node domain"
        );
    }

    /// A node with a name that does not answer for names offers no
    /// hostname, and says which switch is off.
    ///
    /// **Shipped otherwise**: the form asked whether the node had a
    /// domain, the POST asked whether any node would answer for one, and
    /// the refusal told somebody whose node was named to go and name it.
    /// Found on a VM node with `edge` off.
    #[tokio::test]
    async fn a_named_node_that_does_not_answer_for_names_says_so() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;
        crate::infra::node::settings::set_domain(&console.database, Some("localhost"))
            .await
            .expect("a domain");
        crate::infra::network::capability::set_provides(
            &console.database,
            crate::domain::values::capability::Capability::Edge,
            false,
        )
        .await
        .expect("edge off");
        crate::infra::network::ensure_self(&console.database, &console.config)
            .await
            .expect("the row follows");

        let body = console
            .harness
            .get(&format!("{page}/settings/network"))
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("turned off in its capabilities"), "{body}");
        assert!(!body.contains("needs a domain"), "it has one: {body}");

        let refused = console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[
                ("container_port", "80"),
                ("https", "1"),
                ("hostname", "localhost"),
            ])
            .send()
            .await;
        let location = refused.header("location").unwrap_or_default();
        assert!(location.contains("capabilities"), "{location}");
    }

    /// Ticking HTTPS with no hostname must not silently create a port
    /// that serves nothing.
    #[tokio::test]
    async fn https_without_a_hostname_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "80"), ("https", "1")])
            .send()
            .await;

        assert!(ports::all(&console.database)
            .await
            .expect("query")
            .is_empty());
    }

    /// An unticked box is not sent at all, so a hostname left in the
    /// field from a previous attempt must not become a route.
    #[tokio::test]
    async fn a_hostname_without_the_box_ticked_is_ignored() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "80"), ("hostname", "api.example.com")])
            .send()
            .await;

        let stored = ports::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("one port");
        assert_eq!(stored.hostname, None);
    }

    #[tokio::test]
    async fn a_port_can_be_removed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .header("cookie", &cookie)
            .form(&[("container_port", "8080")])
            .send()
            .await;
        let port = ports::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("one port");

        console
            .harness
            .post(&format!("{page}/ports/{}/delete", port.id))
            .header("cookie", &cookie)
            .send()
            .await
            .assert_status(StatusCode::SEE_OTHER);

        assert!(ports::all(&console.database)
            .await
            .expect("query")
            .is_empty());
    }

    #[tokio::test]
    async fn declaring_a_port_needs_a_session() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let page = service(&console, &cookie).await;

        console
            .harness
            .post(&format!("{page}/ports"))
            .form(&[("container_port", "8080")])
            .send()
            .await;

        assert!(ports::all(&console.database)
            .await
            .expect("query")
            .is_empty());
    }

    #[test]
    fn an_empty_port_is_a_service_that_serves_nothing() {
        assert_eq!(parse_port(""), Ok(None));
        assert_eq!(parse_port("80"), Ok(Some(80)));
        assert!(parse_port("0").is_err(), "0 is not a port");
        assert!(parse_port("70000").is_err(), "out of range");
        assert!(parse_port("eighty").is_err());
    }

    /// The mistake this parser exists to avoid: a value with an `=` in
    /// it, which is most connection strings and every base64 secret.
    #[test]
    fn a_value_may_contain_an_equals() {
        let parsed = parse_env("DSN=postgres://u:p@h/db?x=1\nKEY=YWJj==").expect("parsed");
        assert_eq!(
            parsed,
            vec![
                ("DSN".to_string(), "postgres://u:p@h/db?x=1".to_string()),
                ("KEY".to_string(), "YWJj==".to_string()),
            ]
        );
    }

    #[test]
    fn a_pasted_env_file_works_as_pasted() {
        let parsed = parse_env("# comment\n\n  LOG_LEVEL = info  \n").expect("parsed");
        assert_eq!(parsed, vec![("LOG_LEVEL".into(), "info".into())]);
    }

    #[test]
    fn a_line_that_is_not_a_pair_is_refused_by_line_number() {
        let error = parse_env("GOOD=1\nnonsense").expect_err("refused");
        assert!(error.contains("line 2"), "{error}");
    }

    #[test]
    fn a_name_the_shell_cannot_hold_is_refused() {
        let error = parse_env("not a name=1").expect_err("refused");
        assert!(error.contains("variable name"), "{error}");
        assert!(parse_env("=novalue").is_err(), "no name at all");
    }
}
