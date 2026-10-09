//! What the people page posts to.
//!
//! The tests live here rather than beside the pages because what they
//! drive is the POSTs: an invitation is minted, spent, and refused.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::accounts::roles::{NodeRole, ProjectRole};
use crate::accounts::sessions;

use crate::console::auth::{
    back_with_error, field, read_form, see_other, see_other_with_cookie, signed_in,
    SessionMiddleware,
};
use crate::console::ConsoleState;

/// The forms behind those pages.
#[injectable]
pub struct PeopleRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl PeopleRestController {
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

        let asked = crate::features::people::invite_person_request::InvitePersonRequest {
            role: NodeRole::parse(field(&form, "node_role")),
            project: field(&form, "project").to_string(),
            project_role: ProjectRole::parse(field(&form, "project_role")),
        };

        let invited = match crate::features::people::node_user_invite_person::run(
            &self.state.database,
            &account,
            &asked,
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
        // **Through the reveal store, not the query string.** An
        // invitation link is a secret — whoever holds it becomes an
        // account — and it used to travel in the URL, which put it in
        // the address bar, the history and every refresh. The same
        // exception `ConsoleState::reveals` was written for, still open
        // on three pages until this.
        let nonce = self.state.reveals.stash(link);
        Ok(see_other(&format!(
            "/people/shown?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("nonce", &nonce)
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
        let Some(id) = crate::console::auth::segments(&path).get(2).cloned() else {
            return Ok(see_other("/people"));
        };

        match crate::features::people::node_user_revoke_invitation::run(
            &self.state.database,
            &account,
            id,
        )
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
        let Some(id) = crate::console::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
        else {
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

        match crate::features::people::node_user_set_person_role::run(
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
        let Some(id) = crate::console::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
        else {
            return Ok(see_other("/people"));
        };

        match crate::features::people::node_user_remove_person::run(
            &self.state.database,
            &account,
            &id,
        )
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
        let Some(token) = crate::console::auth::segments(&path)
            .get(1)
            .map(|id| id.to_string())
        else {
            return Ok(see_other("/sign-in"));
        };
        let here = format!("/join/{token}");

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let account = match crate::features::people::guest_join_as_node_user::run(
            &self.state.database,
            &crate::console::auth::joining(&token, &form),
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
    use crate::accounts::{self, invitations};
    use crate::console::tests::Console;
    use crate::policies::access;
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
        let link = console.secret_at(location, cookie).await;
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
