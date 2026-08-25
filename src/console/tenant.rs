//! The pages that belong to one organisation, in its own words.
//!
//! ## Why this is a module and not a branch
//!
//! It was a branch for an afternoon, and the words gave it away: an
//! organisation's administrator opened the people page and read "on this
//! node" over a list of their own colleagues, with a role called
//! *Administrator — everything* that meant nothing of the sort. Every
//! sentence about *who runs this* was written for the machine's
//! operator, and conditioning each of them on the scope would have been
//! thirty places to keep in agreement about one distinction.
//!
//! So the pages split. What did **not** split is the answer to "may
//! they": `platform::access` is one function asked by both frontiers,
//! and it stays that way — two page trees is a difference in words, two
//! authorisations is a difference that drifts, and the one that drifts
//! is the one showing somebody a project they were removed from.
//!
//! ## What is shared
//!
//! Projects and services. "Deploy", "Replicas", "Logs" read the same
//! whoever is looking, and those pages are the largest in this console —
//! duplicating them to change nothing would be the drift this module
//! exists to avoid, in the place it would cost most.
//!
//! ## What is absent rather than refused
//!
//! `/node`, `/updates` and `/organisations` are not registered on this
//! router at all, so they 404 here by construction. That is a stronger
//! property than a redirect somebody has to remember to write: there is
//! no handler to forget the check in.

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
use crate::accounts::roles::{NodeRole, ProjectRole};
use crate::accounts::{self, invitations};
use crate::platform::access;

#[injectable]
pub struct TenantPeoplePages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl TenantPeoplePages {
    /// Where the gear lands in this frontier.
    ///
    /// **Its own, because `nodes.rs` is not on this router.** That
    /// controller answers `/settings` for the machine and knows nothing
    /// about organisations — which is what keeps the two halves
    /// separable — so this frontier has to say where its own gear goes.
    /// One page under it today, and a redirect rather than a landing
    /// whose whole content is one link.
    #[view("/settings")]
    #[middleware(SessionMiddleware)]
    async fn settings(&self) -> UiResult<ViewOutcome> {
        match signed_in(&self.auth).is_some_and(|account| account.administers_scope()) {
            true => Ok(Redirect::found("/people").into()),
            false => Ok(Redirect::found("/").into()),
        }
    }

    /// Who is in this organisation, and how somebody else gets in.
    #[view("/people")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !account.administers_scope() {
            return Ok(Redirect::found("/").into());
        }

        let people = accounts::of_scope(&self.state.database, account.tenant_id.as_deref()).await?;
        let invitations = invitations::all(&self.state.database).await?;
        // Their own, which on this router is the only kind there is.
        let projects = access::projects_for(&self.state.database, &account).await?;
        let live: Vec<_> = invitations
            .iter()
            .filter(|invitation| invitation.tenant_id == account.tenant_id)
            .filter(|invitation| invitation.live(crate::platform::now_ms()))
            .collect();

        layout::head("People");
        let frame = Frame::new(&account, Area::Settings, &projects, None, "/people");
        let body = super::language::scoped(account.language, || {
            rsx! {
                (layout::style_tag())
                <h1>(t("People"))</h1>
                <p class="tagline">(t("Everybody in this organisation. What they can reach is its \
                     projects and nothing else on this machine."))</p>

                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                @if let Some(link) = &query.invited {
                    <section class="card stack">
                        <p class="card-label">(t("Send this link"))</p>
                        <p class="field-hint">(t("It works once and expires in seven days. The node will not \
                             show it again — it is not stored in a form anybody can read \
                             back."))</p>
                        <pre><code>(link)</code></pre>
                    </section>
                }

                <section class="stack">
                    <p class="card-label">(t("Accounts"))</p>
                    <table>
                        <thead>
                            <tr>
                                <th>(t("Name"))</th>
                                // Not "on this node": the role is inside
                                // this organisation, and the machine it
                                // happens to run on is none of their
                                // business.
                                <th>(t("In this organisation"))</th>
                                <th></th>
                            </tr>
                        </thead>
                        <tbody>
                            @for person in &people {
                                <tr>
                                    <td>(&person.username)</td>
                                    <td>
                                        @if person.administers_scope() {
                                            (t("Administrator"))
                                        } @else {
                                            (t("Member"))
                                        }
                                    </td>
                                    <td class="row">
                                        @if person.id != account.id {
                                            <form method="post"
                                                  action=(format!("/people/{}/role", person.id))>
                                                <input type="hidden" name="role" value=(
                                                    if person.administers_scope() { "member" } else { "admin" }
                                                )>
                                                <button class="btn btn-secondary btn-sm" type="submit">
                                                    @if person.administers_scope() {
                                                        (t("Make a member"))
                                                    } @else {
                                                        (t("Make an administrator"))
                                                    }
                                                </button>
                                            </form>
                                            <form method="post"
                                                  action=(format!("/people/{}/delete", person.id))>
                                                <button class="btn btn-ghost destructive btn-sm"
                                                        type="submit">
                                                    (t("Remove"))
                                                </button>
                                            </form>
                                        } @else {
                                            <span class="tile-detail">(t("This is you"))</span>
                                        }
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                </section>

                <section class="stack">
                    <p class="card-label">(t("Invite somebody"))</p>
                    <form method="post" action="/people/invite" class="card stack">
                        <label for="node_role">(t("In this organisation"))</label>
                        <select id="node_role" name="node_role">
                            <option value="member">(t("Member — the projects they are added to"))</option>
                            <option value="admin">(t("Administrator — every project of this organisation"))</option>
                        </select>

                        @if !projects.is_empty() {
                            <label for="project">(t("Into a project"))</label>
                            <select id="project" name="project">
                                <option value="">(t("Nothing yet"))</option>
                                @for project in &projects {
                                    <option value=(&project.slug)>(&project.name)</option>
                                }
                            </select>

                            <label for="project_role">(t("As"))</label>
                            <select id="project_role" name="project_role">
                                @for role in ProjectRole::ALL {
                                    <option value=(role.as_str())>(super::language::word(role.label()))</option>
                                }
                            </select>
                        }
                        <p class="field-hint">(t("A link that works once and expires in seven days. \
                             They choose their own username and password."))</p>
                        <div class="actions">
                            <button type="submit">(t("Create invitation"))</button>
                        </div>
                    </form>
                </section>

                @if !live.is_empty() {
                    <section class="stack">
                        <p class="card-label">(t("Invitations waiting"))</p>
                        <table>
                            <thead>
                                <tr><th>(t("In this organisation"))</th><th></th></tr>
                            </thead>
                            <tbody>
                                @for invitation in &live {
                                    <tr>
                                        <td>
                                            @if invitation.node_role == NodeRole::Admin {
                                                (t("Administrator"))
                                            } @else {
                                                (t("Member"))
                                            }
                                        </td>
                                        <td>
                                            <form method="post" action=(
                                                format!("/people/invitations/{}/revoke", invitation.id)
                                            )>
                                                <button class="btn btn-ghost destructive btn-sm"
                                                        type="submit">
                                                    (t("Withdraw"))
                                                </button>
                                            </form>
                                        </td>
                                    </tr>
                                }
                            </tbody>
                        </table>
                    </section>
                }
            }
            .render()
            .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}

#[injectable]
pub struct TenantPeopleApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl TenantPeopleApi {
    /// Mint a link into **this** organisation.
    ///
    /// The scope is never a field here: it is whoever is asking, and
    /// this router only ever answers inside one frontier. `create_into`
    /// refuses anything else regardless — a rule the page arranges is a
    /// courtesy, and the check that counts is on the POST.
    #[post("/people/invite")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn invite(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !account.administers_scope() {
            return Ok(see_other("/"));
        }
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let node_role = match field(&form, "node_role") {
            "admin" => NodeRole::Admin,
            _ => NodeRole::Member,
        };

        let slug = field(&form, "project");
        let project = if slug.is_empty() {
            None
        } else {
            match access::find_project(&self.state.database, &account, slug).await? {
                Some((project, _)) => {
                    Some((project.id, ProjectRole::parse(field(&form, "project_role"))))
                }
                None => return Ok(back_with_error("/people", "no such project")),
            }
        };

        let token = match invitations::create_into(
            &self.state.database,
            &account,
            node_role,
            project.as_ref().map(|(id, role)| (id.as_str(), *role)),
            account.tenant_id.as_deref(),
        )
        .await
        {
            Ok(token) => token,
            Err(error) => return Ok(back_with_error("/people", &error.to_string())),
        };

        // On this organisation's own name, which is the door the account
        // it makes works at — see `ConsoleState::base_url_for`.
        let link = format!(
            "{}/join/{token}",
            self.state.base_url_for(account.tenant_id.as_deref()).await
        );
        Ok(see_other(&format!(
            "/people?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("invited", &link)
                .finish()
        )))
    }

    /// Make somebody an administrator of this organisation, or stop
    /// them being one.
    #[post("/people/:account/role")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_role(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !account.administers_scope() {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/people"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // **Only somebody of this organisation.** The path carries an
        // account id and an id is guessable; without this, one
        // organisation's administrator could promote or demote anybody
        // on the machine. The page lists only their own, and a list is a
        // courtesy.
        if !self.is_ours(&account, id).await? {
            return Ok(back_with_error("/people", "no such person here"));
        }

        let wanted = match field(&form, "role") {
            "admin" => NodeRole::Admin,
            _ => NodeRole::Member,
        };
        match accounts::set_role(&self.state.database, id, wanted).await {
            Ok(()) => Ok(see_other("/people")),
            Err(error) => Ok(back_with_error("/people", &error.to_string())),
        }
    }

    #[post("/people/:account/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !account.administers_scope() {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/people"));
        };
        if !self.is_ours(&account, id).await? {
            return Ok(back_with_error("/people", "no such person here"));
        }
        match accounts::delete(&self.state.database, id).await {
            Ok(()) => Ok(see_other("/people")),
            Err(error) => Ok(back_with_error("/people", &error.to_string())),
        }
    }

    #[post("/people/invitations/:invitation/revoke")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !account.administers_scope() {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(2).cloned() else {
            return Ok(see_other("/people"));
        };
        // Somebody else's invitation is not theirs to withdraw, and an
        // invitation id is as guessable as an account one.
        let ours = invitations::all(&self.state.database)
            .await?
            .into_iter()
            .any(|invitation| invitation.id == id && invitation.tenant_id == account.tenant_id);
        if !ours {
            return Ok(back_with_error("/people", "no such invitation here"));
        }
        invitations::revoke(&self.state.database, id).await?;
        Ok(see_other("/people"))
    }
}

impl TenantPeopleApi {
    /// Whether that account is in the asker's own frontier.
    ///
    /// The one question every write on this page has to ask, in one
    /// place — three spellings of it is how one of them ends up missing.
    async fn is_ours(&self, asking: &accounts::Account, id: &str) -> RestResult<bool> {
        Ok(
            accounts::of_scope(&self.state.database, asking.tenant_id.as_deref())
                .await?
                .into_iter()
                .any(|person| person.id == id),
        )
    }
}
