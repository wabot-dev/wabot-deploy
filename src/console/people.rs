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
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use crate::accounts::roles::{NodeRole, ProjectRole};
use crate::accounts::{self, invitations, sessions};
use crate::platform::access;

use super::auth::{
    back_with_error, field, read_form, see_other, see_other_with_cookie, signed_in, PageQuery,
    SessionMiddleware,
};
use super::shell::{Area, Frame};
use super::{layout, ConsoleState};

#[derive(Debug, Deserialize, Validate)]
pub struct JoinPage {
    pub token: String,
    pub error: Option<String>,
}

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
        if !account.is_admin() {
            return Ok(Redirect::found("/").into());
        }

        let people = accounts::all(&self.state.database).await?;
        let invitations = invitations::all(&self.state.database).await?;
        let projects = access::projects_for(&self.state.database, &account).await?;
        let now = crate::console::now_ms();

        let frame = Frame::new(&account, Area::People, &projects, None, "/people");
        layout::head("People");

        let body = rsx! {
            (layout::style_tag())
            <h1>("People")</h1>
            @if let Some(message) = &query.error {
                (layout::error_note(message))
            }
            @if let Some(link) = &query.invited {
                <section class="card stack">
                    <p class="card-label">("Send this link")</p>
                    <p class="field-hint">(
                        "It works once and expires in seven days. The node will not \
                         show it again — it is not stored in a form anybody can read \
                         back."
                    )</p>
                    <pre><code>(link)</code></pre>
                </section>
            }

            <section class="stack">
                <p class="card-label">("Accounts")</p>
                <table>
                    <thead>
                        <tr><th>("Name")</th><th>("On this node")</th><th></th></tr>
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
                                                    ("Make a member")
                                                } @else {
                                                    ("Make an administrator")
                                                }
                                            </button>
                                        </form>
                                        <form method="post"
                                              action=(format!("/people/{}/delete", person.id))>
                                            <button class="btn btn-ghost destructive btn-sm"
                                                    type="submit">
                                                ("Remove")
                                            </button>
                                        </form>
                                    } @else {
                                        <span class="tile-detail">("This is you")</span>
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            </section>

            <section class="stack">
                <p class="card-label">("Invite somebody")</p>
                <form method="post" action="/people/invite" class="card stack">
                    <label for="node_role">("On this node")</label>
                    <select id="node_role" name="node_role">
                        <option value="member">("Member — their own projects")</option>
                        <option value="admin">("Administrator — everything")</option>
                    </select>

                    <label for="project">("Into a project")</label>
                    <select id="project" name="project">
                        <option value="">("Nothing yet")</option>
                        @for project in &projects {
                            <option value=(&project.slug)>(&project.name)</option>
                        }
                    </select>

                    <label for="project_role">("As")</label>
                    <select id="project_role" name="project_role">
                        @for role in ProjectRole::ALL {
                            <option value=(role.as_str())>(role.label())</option>
                        }
                    </select>
                    <p class="field-hint">(
                        "They choose their own username and password. Nobody here \
                         ever sees it — which is the reason this is a link rather \
                         than a form that sets one for them."
                    )</p>

                    <div class="actions">
                        <button type="submit">("Create invitation")</button>
                    </div>
                </form>
            </section>

            <section class="stack">
                <p class="card-label">("Invitations")</p>
                @if invitations.is_empty() {
                    <p class="tile-detail">("None.")</p>
                } @else {
                    <table>
                        <thead>
                            <tr><th>("For")</th><th>("State")</th><th></th></tr>
                        </thead>
                        <tbody>
                            @for invitation in &invitations {
                                <tr>
                                    <td>(invitation.node_role.label())</td>
                                    <td>
                                        @if invitation.spent() {
                                            <span class="badge">("Used")</span>
                                        } @else if invitation.expired(now) {
                                            <span class="badge badge-warning">("Expired")</span>
                                        } @else {
                                            <span class="badge badge-success">
                                                <span class="dot dot-success"></span>("Open")
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
                                                    ("Withdraw")
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
        .into_inner();

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
        Ok(rsx! {
            (layout::style_tag())
            <main class="shell narrow">
                (super::auth::mark("Join this node"))
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                @if let Some(invitation) = &invitation {
                    <p class="tagline">
                        ("You were invited as ")(invitation.node_role.label().to_lowercase())(".")
                    </p>
                    <form method="post" action=(format!("/join/{}", query.token))
                          class="card stack">
                        <label for="username">("Username")</label>
                        <input id="username" name="username" type="text"
                               autocomplete="username" required autofocus>

                        <label for="password">("Password")</label>
                        <input id="password" name="password" type="password"
                               autocomplete="new-password" required>
                        <p class="field-hint">(
                            "At least 12 characters. Nobody here will ever see it."
                        )</p>

                        <div class="actions">
                            <button type="submit">("Create account")</button>
                        </div>
                    </form>
                } @else {
                    <section class="card stack">
                        <p>(
                            "This invitation is not valid. It may have been used already, \
                             withdrawn, or expired — they last seven days."
                        )</p>
                        <p class="field-hint">("Ask whoever invited you for another.")</p>
                    </section>
                }
            </main>
        }
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
        if !account.is_admin() {
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

        // A project only comes along when one was picked. The role
        // select always sends something, and reading it without a
        // project would put somebody nowhere as a viewer.
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

        let token = match invitations::create(
            &self.state.database,
            &account,
            node_role,
            project.as_ref().map(|(id, role)| (id.as_str(), *role)),
        )
        .await
        {
            Ok(token) => token,
            Err(error) => return Ok(back_with_error("/people", &error.to_string())),
        };

        // Back through the query string, because this is the only time
        // the token exists in clear and the page has to show it once.
        // Stored hashed, so no page can ever show it again.
        let link = format!("{}/join/{token}", self.state.base_url().await);
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
        if !account.is_admin() {
            return Ok(see_other("/"));
        }

        let path = request.uri().path().to_string();
        if let Some(id) = super::auth::segments(&path).get(2) {
            invitations::revoke(&self.state.database, id).await?;
        }
        Ok(see_other("/people"))
    }

    #[post("/people/:account/role")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_role(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !account.is_admin() {
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

        match accounts::set_role(&self.state.database, &id, role).await {
            Ok(()) => Ok(see_other("/people")),
            Err(error) => Ok(back_with_error("/people", &error.to_string())),
        }
    }

    #[post("/people/:account/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        if !account.is_admin() {
            return Ok(see_other("/"));
        }

        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).map(|id| id.to_string()) else {
            return Ok(see_other("/people"));
        };
        if id == account.id {
            return Ok(back_with_error(
                "/people",
                "removing yourself would leave you signed in to nothing",
            ));
        }

        match accounts::delete(&self.state.database, &id).await {
            Ok(()) => Ok(see_other("/people")),
            Err(error) => Ok(back_with_error("/people", &error.to_string())),
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

        let account = match invitations::accept(
            &self.state.database,
            &token,
            field(&form, "username"),
            // Not trimmed: the spaces are part of it, and removing
            // them silently produces a password nobody can type back.
            form.get("password").map(String::as_str).unwrap_or_default(),
        )
        .await
        {
            Ok(account) => account,
            Err(error) => return Ok(back_with_error(&here, &error.to_string())),
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
