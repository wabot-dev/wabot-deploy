//! Organisations, and what each of them may take.
//!
//! ## The node's operator only
//!
//! Making an organisation, giving it a domain and setting its ceiling
//! are decisions about the *machine* — how it is shared out — so they
//! sit beside the node's own settings and ask `is_admin`, which a
//! tenant's administrator is not. What a tenant's administrator manages
//! is what is inside their frontier: their projects and their people.
//!
//! ## The domain is the frontier
//!
//! A tenant's console answers on its own hostname, and `edge::dispatch`
//! stamps the scope on the request before any handler runs — so the
//! narrowing is the route rather than something thirty queries have to
//! remember. Setting the domain here is therefore what brings a tenant's
//! console into existence at all; without one it has projects and
//! nowhere to administer them from.

use std::sync::Arc;

use hypertext::prelude::*;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use super::auth::{
    back_with_error, field, read_form, see_other, signed_in, PageQuery, SessionMiddleware,
};
use super::language::t;
use super::shell::{Area, Frame};
use super::{layout, ConsoleState};
use crate::platform::{access, presets, tenants};

#[injectable]
pub struct TenantPages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl TenantPages {
    /// Every organisation on this node, what it answers to and what it
    /// may take.
    #[view("/organisations")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(Redirect::found("/").into());
        }

        // The head, or the page renders as a document with no
        // stylesheet — nav, content and all, in Times New Roman. It is
        // what every other view does first and the one thing a new page
        // has nothing reminding it of.
        layout::head("Organisations");

        let rows = tenants::all(&self.state.database).await?;
        let mut cards = Vec::new();
        for tenant in &rows {
            let held = access::of_tenant(&self.state.database, &tenant.id)
                .await
                .unwrap_or_default();
            cards.push((tenant.clone(), held));
        }

        let frame = Frame::new(&account, Area::Settings, &[], None, "/organisations");
        let body = super::language::scoped(account.language, || {
            rsx! {
                // The console's own CSS, which every view emits into its
                // own body. `wabot.css` is the design system's tokens;
                // the shell, the cards and this page's layout are in
                // here, and a page without it renders complete and
                // unstyled — which is exactly how it looked.
                (layout::style_tag())
                <div class="stack">
                    <div class="split">
                        <div class="stack-sm">
                            <h1>(t("Organisations"))</h1>
                            <p class="slug-preview">(t("Who this node is shared out to"))</p>
                        </div>
                    </div>
                    @if let Some(message) = &query.error {
                        (layout::error_note(message))
                    }
                    // The one moment the token exists in clear, shown by
                    // the page that made it and never again — the same
                    // block the people page has, because an invitation
                    // minted here has to be readable here.
                    @if let Some(link) = &query.invited {
                        <section class="card stack">
                            <p class="card-label">(t("Send this link"))</p>
                            <p class="field-hint">(t("It works once and expires in seven days. The node will not \
                                 show it again — it is not stored in a form anybody can read \
                                 back."))</p>
                            <pre><code>(link)</code></pre>
                        </section>
                    }

                    <section class="card stack">
                        <p class="card-label">(t("New organisation"))</p>
                        <p class="field-hint">(t("Its projects and its people are its own: an \
                             administrator of one reaches nothing outside it, and this \
                             node's operator still sees everything the machine is \
                             running. Give it a domain to bring up its console."))</p>
                        <form method="post" action="/organisations">
                            <label for="name">(t("Name"))</label>
                            <input id="name" name="name" autocomplete="off" required>
                            <div class="actions">
                                <button type="submit">(t("Create"))</button>
                            </div>
                        </form>
                    </section>

                    @for (tenant, held) in &cards {
                        <section class="card stack">
                            <p class="card-label">(&tenant.name)</p>
                            <p class="slug-preview">(&tenant.slug)(" · ")(held.len().to_string())(t(" project(s)"))</p>

                            <form method="post" class="stack"
                                  action=(format!("/organisations/{}/domain", tenant.id))>
                                <label for=(format!("domain-{}", tenant.id))>(t("Domain"))</label>
                                <input id=(format!("domain-{}", tenant.id)) name="domain"
                                       autocomplete="off"
                                       value=(tenant.domain.clone().unwrap_or_default())>
                                <p class="field-hint">(t("Where its console answers, and the suffix its \
                                     service names hang off. It has to point at this node, \
                                     and it cannot be one of this node's own names — the \
                                     console an operator fixes things from must not be \
                                     reachable only through somebody else's."))</p>
                                <div class="actions">
                                    <button type="submit">(t("Save domain"))</button>
                                </div>
                            </form>

                            <form method="post" class="stack"
                                  action=(format!("/organisations/{}/quota", tenant.id))>
                                <label for=(format!("memory-{}", tenant.id))>(t("Memory"))</label>
                                <input id=(format!("memory-{}", tenant.id)) name="memory"
                                       autocomplete="off"
                                       placeholder=(t("no ceiling"))
                                       value=(tenant.quota_memory.map(crate::node::memory::human).unwrap_or_default())>
                                <label for=(format!("cpu-{}", tenant.id))>(t("CPU"))</label>
                                // **`cores_field`, not `cpu_label`.** One is what a
                                // number is called on a page and the other is what
                                // goes back into the box that produced it — and the
                                // difference bites: `2 cores` read back as `2`, but
                                // a value of `2000 cores` is what somebody typing
                                // millicores into an unlabelled box gets, and it
                                // reads back as two thousand of them.
                                <input id=(format!("cpu-{}", tenant.id)) name="cpu"
                                       autocomplete="off"
                                       placeholder=(t("no ceiling"))
                                       value=(tenant.quota_cpu.map(presets::cores_field).unwrap_or_default())>
                                // The unit, said the way the service's own resources
                                // form says it. Without this the box took "2000" as
                                // two thousand cores, which is what it means and not
                                // what anybody typing it meant.
                                <p class="field-hint">(t("Empty is no ceiling. Memory like 512 MB or 2 GB; \
                                     CPU in cores like 0.5 or 2, or millicores with an m, like 500m. \
                                     What a service may take is what counts against this, summed \
                                     over every copy wherever it runs — the limit is the \
                                     reservation, here as everywhere else."))</p>
                                <div class="actions">
                                    <button type="submit">(t("Save quota"))</button>
                                </div>
                            </form>

                            // **The invitation lives here, not on the
                            // people page.** It was a selector there for
                            // one afternoon and the form stopped making
                            // sense: the role read "on this node" while
                            // naming an organisation, and the project
                            // list was the node's rather than theirs.
                            // Two of the three questions change meaning
                            // with the scope, and without JavaScript a
                            // list cannot follow a select.
                            //
                            // So the scope is *where the form is*, which
                            // is the shape this console already uses —
                            // placement lives on the service, edges live
                            // on the service. Here the project list is
                            // that organisation's by construction and
                            // the words can say what they mean.
                            <form method="post" class="stack"
                                  action=(format!("/organisations/{}/invite", tenant.id))>
                                <p class="card-label">(t("Invite somebody to this organisation"))</p>
                                <label for=(format!("role-{}", tenant.id))>(t("In it"))</label>
                                <select id=(format!("role-{}", tenant.id)) name="node_role">
                                    <option value="member">(t("Member — the projects they are added to"))</option>
                                    <option value="admin">(t("Administrator — every project of this organisation"))</option>
                                </select>
                                @if held.is_empty() {
                                    <p class="field-hint">(t("It has no projects yet. Whoever you invite as \
                                         an administrator can make the first one."))</p>
                                } @else {
                                    <label for=(format!("project-{}", tenant.id))>(t("Into a project"))</label>
                                    <select id=(format!("project-{}", tenant.id)) name="project">
                                        <option value="">(t("Nothing yet"))</option>
                                        @for project in held {
                                            <option value=(&project.slug)>(&project.name)</option>
                                        }
                                    </select>
                                    <label for=(format!("project-role-{}", tenant.id))>(t("As"))</label>
                                    <select id=(format!("project-role-{}", tenant.id)) name="project_role">
                                        @for role in crate::accounts::roles::ProjectRole::ALL {
                                            <option value=(role.as_str())>(super::language::word(role.label()))</option>
                                        }
                                    </select>
                                }
                                <p class="field-hint">(t("A link that works once and expires in seven days. \
                                     They choose their own username and password."))</p>
                                // **The link lands on the organisation's
                                // own name**, because that is the door
                                // their account works at — the node's
                                // would sign them up and bounce them on
                                // the first click. Without a domain there
                                // is no such door yet, and saying so
                                // beats handing over a link that appears
                                // to work.
                                @if tenant.domain.is_none() {
                                    <p class="field-hint">(t("It has no domain yet, so the link will be on \
                                         this node's name — the account is made, and the console it \
                                         belongs to exists once you give the organisation one."))</p>
                                }
                                <div class="actions">
                                    <button type="submit">(t("Create invitation"))</button>
                                </div>
                            </form>

                            <form method="post" action=(format!("/organisations/{}/delete", tenant.id))>
                                <button class="btn btn-ghost destructive" type="submit">
                                    (t("Delete organisation"))
                                </button>
                            </form>
                        </section>
                    }
                </div>
            }
            .render()
            .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}

#[injectable]
pub struct TenantApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl TenantApi {
    #[post("/organisations")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        match tenants::create(&self.state.database, field(&form, "name")).await {
            Ok(_) => Ok(see_other("/organisations")),
            Err(error) => Ok(back_with_error("/organisations", &error.to_string())),
        }
    }

    /// What its console answers to.
    ///
    /// Refused when it is one of this node's own names, and the check is
    /// here as well as in `routing::sync` on purpose: the sync skipping
    /// it would leave a row saved on a page that shows it and a console
    /// that never answers there, which is a setting that lies.
    #[post("/organisations/:tenant/domain")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_domain(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let wanted = field(&form, "domain").trim().to_lowercase();
        // By id or slug, the way every other page here accepts either —
        // and it is also what turns a stale link into "no such
        // organisation" rather than a silent no-op.
        let Ok(Some(tenant)) = tenants::find(&self.state.database, id).await else {
            return Ok(back_with_error("/organisations", "no such organisation"));
        };
        let id = &tenant.id;

        if !wanted.is_empty() {
            let ours =
                crate::node::settings::domain(&self.state.database, &self.state.config).await;
            if ours.as_deref() == Some(wanted.as_str()) {
                return Ok(back_with_error(
                    "/organisations",
                    "that is this node's own name — the console an operator fixes things from cannot be somebody else's frontier",
                ));
            }
            // And not a name another organisation already answers on: two
            // rows claiming one `Host` is not something a request can
            // resolve, and choosing silently would make the wrong one
            // look right.
            if let Ok(Some(held)) = tenants::by_domain(&self.state.database, &wanted).await {
                if &held.id != id {
                    return Ok(back_with_error(
                        "/organisations",
                        &format!("{} already answers on that name", held.name),
                    ));
                }
            }
        }

        let domain = (!wanted.is_empty()).then_some(wanted);
        if let Err(error) = tenants::set_domain(&self.state.database, id, domain.as_deref()).await {
            return Ok(back_with_error("/organisations", &error.to_string()));
        }
        // The route follows the row, here rather than at the next
        // deployment: a domain saved and not routed is a console that
        // answers 404 on the name the page says it is at.
        self.state.deployer.sync_routes().await;
        Ok(see_other("/organisations"))
    }

    #[post("/organisations/:tenant/quota")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_quota(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // Empty is no ceiling, which is a real answer and not a missing
        // field: an organisation whose quota is lifted has to be able to
        // say so.
        let memory = match presets::parse_size(field(&form, "memory")) {
            Ok(memory) => memory,
            Err(error) => return Ok(back_with_error("/organisations", &error)),
        };
        let cpu = match presets::parse_cores(field(&form, "cpu")) {
            Ok(cpu) => cpu,
            Err(error) => return Ok(back_with_error("/organisations", &error)),
        };

        match tenants::set_quota(&self.state.database, id, memory, cpu, None).await {
            Ok(()) => Ok(see_other("/organisations")),
            Err(error) => Ok(back_with_error("/organisations", &error.to_string())),
        }
    }

    /// Mint a link into one organisation.
    ///
    /// **Here rather than on the people page**, and that is the split
    /// Jorge asked for made real: `console::people` is the machine's
    /// own, knows nothing about organisations, and would keep working if
    /// this file were deleted. Everything about tenancy that the
    /// operator does lives in this controller.
    ///
    /// It is also the only way into an organisation, since one with
    /// nobody in it cannot invite anybody either.
    #[post("/organisations/:tenant/invite")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn invite(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let Ok(Some(tenant)) = tenants::find(&self.state.database, id).await else {
            return Ok(back_with_error("/organisations", "no such organisation"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let node_role = match field(&form, "node_role") {
            "admin" => crate::accounts::roles::NodeRole::Admin,
            _ => crate::accounts::roles::NodeRole::Member,
        };

        let slug = field(&form, "project");
        let project = if slug.trim().is_empty() {
            None
        } else {
            match access::find_project(&self.state.database, &account, slug).await? {
                // **Inside this organisation.** An operator sees every
                // project the machine holds, so without this a link into
                // one could carry membership of another's — somebody who
                // reaches exactly one project and no console that admits
                // they exist. The form lists the right ones; a list is a
                // courtesy.
                Some((project, _)) if project.tenant_id.as_deref() == Some(&*tenant.id) => Some((
                    project.id,
                    crate::accounts::roles::ProjectRole::parse(field(&form, "project_role")),
                )),
                Some(_) => {
                    return Ok(back_with_error(
                        "/organisations",
                        "that project is not in this organisation",
                    ))
                }
                None => return Ok(back_with_error("/organisations", "no such project")),
            }
        };

        let token = match crate::accounts::invitations::create_into(
            &self.state.database,
            &account,
            node_role,
            project.as_ref().map(|(id, role)| (id.as_str(), *role)),
            Some(&tenant.id),
        )
        .await
        {
            Ok(token) => token,
            Err(error) => return Ok(back_with_error("/organisations", &error.to_string())),
        };

        // On the organisation's own name, which is the door the account
        // it makes works at — the node's would sign them up and bounce
        // them on their first click.
        let link = format!(
            "{}/join/{token}",
            self.state.base_url_for(Some(&tenant.id)).await
        );
        Ok(see_other(&format!(
            "/organisations?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("invited", &link)
                .finish()
        )))
    }

    #[post("/organisations/:tenant/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        match tenants::delete(&self.state.database, id).await {
            Ok(()) => {
                // Its route goes with it, or the name keeps answering
                // for a frontier that no longer exists.
                self.state.deployer.sync_routes().await;
                Ok(see_other("/organisations"))
            }
            Err(error) => Ok(back_with_error("/organisations", &error.to_string())),
        }
    }
}
