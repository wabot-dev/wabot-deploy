//! People: who is on the node, and how somebody else gets here.
//!
//! ## The join page is the only one a stranger may open
//!
//! Everything else on the console needs a session. `/join/<token>`
//! cannot: the person opening it has no account yet, which is the
//! point. What guards it is the token — one use, time-limited, stored
//! hashed — exactly like the setup token, and for the same reasons.

use std::sync::Arc;

use hypertext::prelude::*;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use crate::accounts::roles::{NodeRole, ProjectRole};
use crate::accounts::{self, invitations, sessions};
use crate::policies::access;
use crate::views::texts::language::t;

use super::auth::{
    back_with_error, field, read_form, see_other, see_other_with_cookie, signed_in, JoinPage,
    PageQuery, SessionMiddleware,
};
use super::ConsoleState;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};

#[injectable]
pub struct PeoplePages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl PeoplePages {
    /// Everybody on the node, and every invitation.
    ///
    /// Administrators only. Who else is here, and who has a live link
    /// to become somebody here, is the node's own business.
    #[view("/people")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(Redirect::found("/").into());
        }

        // **This machine's own operators**, and nobody else's people.
        // An organisation's are managed from its own console, by a
        // different controller — see `console::tenant`. This page knows
        // nothing about organisations, which is what lets that whole
        // half be deleted without touching it.
        let people = accounts::of_scope(&self.state.database, None).await?;

        let invitations = invitations::all(&self.state.database).await?;
        let projects = access::projects_for(&self.state.database, &account).await?;
        let now = crate::console::now_ms();

        let frame = Frame::new(&account, Area::Settings, &projects, None, "/people");
        layout::head("People");

        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let body = crate::views::texts::language::scoped(account.language, || {
            rsx! {
            (layout::style_tag())
            <h1>(t("People"))</h1>
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
                        <tr><th>(t("Name"))</th><th>(t("On this node"))</th><th></th></tr>
                    </thead>
                    <tbody>
                        @for person in &people {
                            <tr>
                                <td>(&person.username)</td>
                                <td>(person.role.label())</td>
                                <td class="row">
                                    @if person.id != account.id {
                                        <form method="post"
                                              action=(format!("/people/{}/role", person.id))>
                                            <input type="hidden" name="role" value=(
                                                if person.is_admin() { "member" } else { "admin" }
                                            )>
                                            <button class="btn btn-secondary btn-sm" type="submit">
                                                @if person.is_admin() {
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
                    <label for="node_role">(t("On this node"))</label>
                    <select id="node_role" name="node_role">
                        <option value="member">(t("Member — their own projects"))</option>
                        <option value="admin">(t("Administrator — everything"))</option>
                    </select>

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
                            <option value=(role.as_str())>(role.label())</option>
                        }
                    </select>
                    <p class="field-hint">(t("They choose their own username and password. Nobody here \
                         ever sees it — which is the reason this is a link rather \
                         than a form that sets one for them."))</p>

                    <div class="actions">
                        <button type="submit">(t("Create invitation"))</button>
                    </div>
                </form>
            </section>

            <section class="stack">
                <p class="card-label">(t("Invitations"))</p>
                @if invitations.is_empty() {
                    <p class="tile-detail">(t("None."))</p>
                } @else {
                    <table>
                        <thead>
                            <tr><th>(t("For"))</th><th>(t("State"))</th><th></th></tr>
                        </thead>
                        <tbody>
                            @for invitation in &invitations {
                                <tr>
                                    <td>
                                        (invitation.node_role.label())
                                        @if let Some(name) = &invitation.scope_name {
                                            <div class="tile-detail">
                                                (t("Organisation")) " " (name)
                                            </div>
                                        }
                                    </td>
                                    <td>
                                        @if invitation.spent() {
                                            <span class="badge">(t("Used"))</span>
                                        } @else if invitation.expired(now) {
                                            <span class="badge badge-warning">(t("Expired"))</span>
                                        } @else {
                                            <span class="badge badge-success">
                                                <span class="dot dot-success"></span>(t("Open"))
                                            </span>
                                        }
                                    </td>
                                    <td>
                                        @if !invitation.spent() {
                                            <form method="post"
                                                  action=(format!(
                                                      "/people/invitations/{}/revoke",
                                                      invitation.id
                                                  ))>
                                                <button class="btn btn-ghost destructive btn-sm"
                                                        type="submit">
                                                    (t("Withdraw"))
                                                </button>
                                            </form>
                                        }
                                    </td>
                                </tr>
                            }
                        </tbody>
                    </table>
                }
            </section>
        }
            .render()
            .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// Accept an invitation. The one page a stranger may open.
    #[view("/join/:token")]
    #[middleware(SessionMiddleware)]
    async fn join(&self, query: JoinPage) -> UiResult<ViewOutcome> {
        // Somebody already signed in following an invitation link is
        // almost certainly the wrong person at the right machine.
        // Their session is not what the link is for.
        if signed_in(&self.auth).is_some() {
            return Ok(Redirect::found("/?error=sign+out+first+to+accept+an+invitation").into());
        }

        let invitation = invitations::look_up(&self.state.database, &query.token).await?;

        layout::head("Join");
        let page = crate::views::pages::guest_join_page::Join {
            heading: "Join this node",
            token: &query.token,
            invitation: invitation.as_ref(),
            error: query.error.as_deref(),
        };
        Ok(crate::views::pages::guest_join_page::render(&page)
            .into_view()
            .into())
    }
}

/// The forms behind those pages.
#[injectable]
pub struct PeopleApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl PeopleApi {
    #[post("/people/invite")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn invite(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(see_other("/"));
        }
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked = crate::usecases::node_user_invite_person::Asked {
            role: match field(&form, "node_role") {
                "admin" => NodeRole::Admin,
                _ => NodeRole::Member,
            },
            project: Some(field(&form, "project")),
            project_role: ProjectRole::parse(field(&form, "project_role")),
        };

        let invited = match crate::usecases::node_user_invite_person::run(
            &self.state.database,
            &account,
            asked,
        )
        .await
        {
            Ok(invited) => invited,
            Err(reason) => return Ok(back_with_error("/people", &reason)),
        };

        // The only time the token exists in clear, and the page has to
        // show it once. Stored hashed, so no page can ever show it again.
        let link = format!(
            "{}/join/{}",
            self.state.base_url_for(None).await,
            invited.token
        );
        Ok(see_other(&format!(
            "/people?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("invited", &link)
                .finish()
        )))
    }

    #[post("/people/invitations/:invitation/revoke")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn revoke(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(see_other("/"));
        }

        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(2).cloned() else {
            return Ok(see_other("/people"));
        };

        match crate::usecases::node_user_revoke_invitation::run(&self.state.database, &account, id)
            .await
        {
            Ok(()) => Ok(see_other("/people")),
            Err(reason) => Ok(back_with_error("/people", &reason)),
        }
    }

    #[post("/people/:account/role")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_role(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(see_other("/people"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let role = match field(&form, "role") {
            "admin" => NodeRole::Admin,
            _ => NodeRole::Member,
        };

        match crate::usecases::node_user_set_person_role::run(
            &self.state.database,
            &account,
            &id,
            role,
        )
        .await
        {
            Ok(()) => Ok(see_other("/people")),
            Err(reason) => Ok(back_with_error("/people", &reason)),
        }
    }

    #[post("/people/:account/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(see_other("/"));
        }
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(see_other("/people"));
        };

        match crate::usecases::node_user_remove_person::run(&self.state.database, &account, &id)
            .await
        {
            Ok(()) => Ok(see_other("/people")),
            Err(reason) => Ok(back_with_error("/people", &reason)),
        }
    }

    /// Accept an invitation: create the account and sign them in.
    #[post("/join/:token")]
    #[raw]
    async fn join(&self, request: Request) -> RestResult<Response> {
        let path = request.uri().path().to_string();
        let Some(token) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(see_other("/sign-in"));
        };
        let here = format!("/join/{token}");

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let account = match crate::usecases::guest_join_as_node_user::run(
            &self.state.database,
            crate::usecases::guest_join_as_node_user::Asked {
                token: &token,
                username: field(&form, "username"),
                // Not trimmed: the spaces are part of it, and removing
                // them silently produces a password nobody can type back.
                password: form.get("password").map(String::as_str).unwrap_or_default(),
            },
        )
        .await
        {
            Ok(account) => account,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        // Signed in straight away: they just proved they hold the
        // invitation and chose a password, and asking them to type it
        // again is the same secret twice in a row.
        match sessions::create(&self.state.database, &account).await {
            Ok(token) => Ok(see_other_with_cookie("/", sessions::set_cookie(&token))),
            Err(error) => {
                tracing::error!(%error, "could not start a session after joining");
                Ok(see_other("/sign-in"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;
    use wabot::rest::axum::http::StatusCode;

    /// Create an invitation the way the console does, and pull the
    /// token back out of the link it shows.
    async fn invite(console: &Console, cookie: &str, fields: &[(&str, &str)]) -> String {
        let response = console
            .harness
            .post("/people/invite")
            .header("cookie", cookie)
            .form(fields)
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);

        let location = response.header("location").expect("redirected");
        let query = location.split_once('?').expect("carries the link").1;
        let link = form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "invited")
            .map(|(_, value)| value.into_owned())
            .expect("the link");
        link.rsplit('/').next().expect("the token").to_string()
    }

    #[tokio::test]
    async fn an_invitation_brings_somebody_in_and_signs_them_in() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let token = invite(&console, &cookie, &[("node_role", "member")]).await;

        let response = console
            .harness
            .post(&format!("/join/{token}"))
            .form(&[
                ("username", "colleague"),
                ("password", "a long passphrase here"),
            ])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert_eq!(response.header("location"), Some("/"));
        assert!(
            response.header("set-cookie").is_some(),
            "they are signed in, not sent to type the password again"
        );

        let people = accounts::all(&console.database).await.expect("query");
        assert_eq!(people.len(), 2);
    }

    /// The page behind a used link has to say so rather than show a
    /// form that will be refused.
    #[tokio::test]
    async fn a_spent_invitation_shows_no_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let token = invite(&console, &cookie, &[("node_role", "member")]).await;

        console
            .harness
            .post(&format!("/join/{token}"))
            .form(&[
                ("username", "first"),
                ("password", "a long passphrase here"),
            ])
            .send()
            .await;

        let body = console
            .harness
            .get(&format!("/join/{token}"))
            .send()
            .await
            .body;
        assert!(body.contains("not valid"), "{body}");
        assert!(!body.contains("name=\"password\""), "no form to fill in");
    }

    /// The whole point of the people page: only an administrator sees
    /// who is on the node.
    #[tokio::test]
    async fn a_member_cannot_open_the_people_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let token = invite(&console, &cookie, &[("node_role", "member")]).await;

        let joined = console
            .harness
            .post(&format!("/join/{token}"))
            .form(&[
                ("username", "member"),
                ("password", "a long passphrase here"),
            ])
            .send()
            .await;
        let member_cookie = joined
            .header("set-cookie")
            .expect("signed in")
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let response = console
            .harness
            .get("/people")
            .header("cookie", &member_cookie)
            .send()
            .await;
        assert_eq!(response.header("location"), Some("/"));

        // And the endpoints behind it, which are the real boundary.
        let refused = console
            .harness
            .post("/people/invite")
            .header("cookie", &member_cookie)
            .form(&[("node_role", "admin")])
            .send()
            .await;
        assert_eq!(refused.header("location"), Some("/"));
        assert_eq!(
            invitations::all(&console.database)
                .await
                .expect("query")
                .len(),
            1,
            "the one the administrator made, and no more"
        );
    }

    #[tokio::test]
    async fn the_only_administrator_cannot_be_demoted_or_removed() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let admin = accounts::all(&console.database)
            .await
            .expect("query")
            .pop()
            .expect("the admin");

        // Their own row is refused before it reaches the account layer,
        // because signing yourself out of your own node is never what
        // was meant.
        let response = console
            .harness
            .post(&format!("/people/{}/delete", admin.id))
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(response
            .header("location")
            .is_some_and(|to| to.contains("error=")));
        assert_eq!(
            accounts::all(&console.database).await.expect("query").len(),
            1
        );
    }

    /// An invitation into a project is one link rather than a link and
    /// a second step somebody forgets.
    #[tokio::test]
    async fn an_invitation_can_carry_a_project() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/projects")
            .header("cookie", &cookie)
            .form(&[("name", "shared")])
            .send()
            .await;

        let token = invite(
            &console,
            &cookie,
            &[
                ("node_role", "member"),
                ("project", "shared"),
                ("project_role", "deployer"),
            ],
        )
        .await;

        console
            .harness
            .post(&format!("/join/{token}"))
            .form(&[
                ("username", "colleague"),
                ("password", "a long passphrase here"),
            ])
            .send()
            .await;

        let member = accounts::all(&console.database)
            .await
            .expect("query")
            .into_iter()
            .find(|account| account.username == "colleague")
            .expect("joined");
        let projects = access::projects_for(&console.database, &member)
            .await
            .expect("query");
        assert_eq!(projects.len(), 1, "they landed in the project");
    }

    /// Entering the console replaces the whole document: the shell, the
    /// navigation and everything that depends on a session appear for
    /// the first time. A boosted submit swaps a fragment into the page
    /// that is already there, which for these forms is the page nobody
    /// should still be looking at.
    #[tokio::test]
    async fn the_forms_that_let_somebody_in_are_not_boosted() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let token = invite(&console, &cookie, &[("node_role", "member")]).await;

        for path in ["/sign-in".to_string(), format!("/join/{token}")] {
            let body = console.harness.get(&path).send().await.body;
            assert!(
                body.contains("<form") && body.contains("data-wabot-reload"),
                "{path} has a form a boosted submit would swap under"
            );
        }
    }

    /// Somebody already signed in is the wrong person at the right
    /// machine.
    #[tokio::test]
    async fn a_signed_in_visitor_is_not_offered_the_join_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let token = invite(&console, &cookie, &[("node_role", "member")]).await;

        let response = console
            .harness
            .get(&format!("/join/{token}"))
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(response
            .header("location")
            .is_some_and(|to| to.starts_with("/?error=")));
    }
}
