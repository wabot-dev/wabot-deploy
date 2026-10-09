//! What the two doors post to, and the switches that belong to an
//! account rather than to a page.

use crate::infra::accounts::sessions;
use crate::web::console::ConsoleState;
use std::sync::Arc;
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::web::console::auth::*;

// ---------- form submissions ---------------------------------------------

/// The POSTs behind those two pages, and sign-out.
///
/// `#[raw]` throughout: what a form needs back is a 303 with a
/// `Set-Cookie` on it, and the JSON path cannot express either.
#[injectable]
pub struct AuthRestController {
    state: Arc<ConsoleState>,
    /// Only `/theme` needs it — it writes against whoever is asking.
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl AuthRestController {
    #[post("/setup")]
    #[raw]
    async fn create_admin(&self, request: Request) -> RestResult<Response> {
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::features::access::guest_claim_the_node::run(
            &self.state.database,
            field(&form, "setup_token"),
            field(&form, "username"),
            form.get("password").map(String::as_str).unwrap_or_default(),
        )
        .await
        {
            Ok(token) => Ok(see_other_with_cookie("/", sessions::set_cookie(&token))),
            Err(reason) => Ok(back_with_error("/setup", &reason)),
        }
    }

    #[post("/sign-in")]
    #[raw]
    async fn sign_in(&self, request: Request) -> RestResult<Response> {
        // **Which frontier they are signing in to**, decided by the
        // `Host` before this ran and read here rather than asked for: a
        // username belongs to a scope, so three people called `jorge`
        // is the ordinary case and a search over the whole table would
        // find whichever came first. See migration `0057`.
        let scope = request
            .extensions()
            .get::<crate::infra::edge::Tenancy>()
            .map(|tenancy| tenancy.0.clone());

        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::features::access::guest_sign_in::run(
            &self.state.database,
            field(&form, "username"),
            form.get("password").map(String::as_str).unwrap_or_default(),
            scope.as_deref(),
        )
        .await
        {
            Ok(token) => Ok(see_other_with_cookie("/", sessions::set_cookie(&token))),
            Err(reason) => Ok(back_with_error("/sign-in", &reason)),
        }
    }

    /// End this session.
    ///
    /// Revokes the row as well as clearing the cookie: a cookie the
    /// browser forgot is still a cookie somebody copied.
    /// Which theme to read in.
    ///
    /// A form with three submits, so choosing is one click and the
    /// current answer is visible rather than folded into a menu. The
    /// value lands on the account, which means the next page the
    /// server renders already carries it — no script, and no flash of
    /// the theme somebody just turned off.
    #[post("/theme")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_theme(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let theme = crate::domain::values::theme::Theme::parse(field(&form, "theme"));
        if let Err(error) =
            crate::infra::accounts::set_theme(&self.state.database, &account.id, theme).await
        {
            tracing::warn!(%error, "could not store the theme");
        }

        // Back where they were. A path from the form rather than the
        // Referer: this console decides where its own links go, and a
        // header a client controls is not that.
        let back = field(&form, "from");
        Ok(see_other(if back.starts_with('/') { back } else { "/" }))
    }

    /// Which language they read in. The theme's twin, and deliberately
    /// its twin: same store, same "back where they were", same one
    /// click.
    #[post("/language")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_language(&self, request: Request) -> RestResult<Response> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(see_other("/sign-in"));
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let language = crate::domain::values::language::Language::parse(field(&form, "language"));
        if let Err(error) =
            crate::infra::accounts::set_language(&self.state.database, &account.id, language).await
        {
            tracing::warn!(%error, "could not store the language");
        }

        let back = field(&form, "from");
        Ok(see_other(if back.starts_with('/') { back } else { "/" }))
    }

    #[post("/sign-out")]
    #[raw]
    async fn sign_out(&self, request: Request) -> RestResult<Response> {
        if let Some(token) = sessions::from_headers(request.headers()) {
            if let Err(error) = sessions::revoke(&self.state.database, &token).await {
                tracing::warn!(%error, "could not revoke the session");
            }
        }
        Ok(Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, "/sign-in")
            .header(header::SET_COOKIE, sessions::clear_cookie())
            .body(Body::empty())
            .expect("a constant response is well-formed"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::console::tests::Console;
    use std::collections::HashMap;

    // ---------- the flow over HTTP ---------------------------------------

    #[tokio::test]
    async fn a_stranger_is_sent_to_setup_and_setup_signs_them_in() {
        let console = Console::new().await;

        let response = console.harness.get("/").send().await;
        response.assert_status(StatusCode::FOUND);
        assert_eq!(response.header("location"), Some("/setup"));

        let page = console.harness.get("/setup").send().await;
        page.assert_ok();
        assert!(page.body.contains("setup_token"), "{}", page.body);

        let cookie = console.signed_in().await;
        let home = console
            .harness
            .get("/")
            .header("cookie", &cookie)
            .send()
            .await;
        home.assert_ok();
        assert!(home.body.contains("Projects"), "{}", home.body);
    }

    /// The token is spent on use. A second setup — with the same token
    /// or any other — is how a node gets taken over by whoever finds an
    /// old terminal buffer.
    #[tokio::test]
    async fn setup_cannot_happen_twice() {
        let console = Console::new().await;
        console.signed_in().await;

        let response = console
            .harness
            .post("/setup")
            .form(&[
                ("username", "intruder"),
                ("password", "another long passphrase"),
                ("setup_token", &console.setup_token),
            ])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        assert!(
            response
                .header("location")
                .is_some_and(|to| to.starts_with("/setup?error=")),
            "refused with a reason: {:?}",
            response.header("location")
        );
        assert!(
            response.header("set-cookie").is_none(),
            "and nobody was signed in"
        );

        // The page itself is gone too.
        let page = console.harness.get("/setup").send().await;
        assert_eq!(page.header("location"), Some("/sign-in"));
    }

    #[tokio::test]
    async fn signing_in_and_out_moves_the_session() {
        let console = Console::new().await;
        console.signed_in().await;

        let response = console
            .harness
            .post("/sign-in")
            .form(&[("username", "jorge"), ("password", "correct horse battery")])
            .send()
            .await;
        response.assert_status(StatusCode::SEE_OTHER);
        let cookie = response
            .header("set-cookie")
            .expect("a session")
            .split(';')
            .next()
            .unwrap()
            .to_string();

        let out = console
            .harness
            .post("/sign-out")
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(out
            .header("set-cookie")
            .is_some_and(|value| value.contains("Max-Age=0")));

        // Revoked server-side, not just forgotten by the browser.
        let after = console
            .harness
            .get("/")
            .header("cookie", &cookie)
            .send()
            .await;
        assert_eq!(after.header("location"), Some("/sign-in"));
    }

    /// A wrong username and a wrong password must be indistinguishable,
    /// or the form is a list of which accounts exist.
    #[tokio::test]
    async fn a_wrong_password_and_an_unknown_user_answer_the_same() {
        let console = Console::new().await;
        console.signed_in().await;

        let mut answers = Vec::new();
        for (username, password) in [
            ("jorge", "wrong password here"),
            ("nobody", "wrong password here"),
        ] {
            let response = console
                .harness
                .post("/sign-in")
                .form(&[("username", username), ("password", password)])
                .send()
                .await;
            assert!(response.header("set-cookie").is_none());
            answers.push(response.header("location").unwrap_or_default().to_string());
        }
        assert_eq!(answers[0], answers[1], "the two are told apart");
    }

    #[test]
    fn a_mutation_answers_303_so_a_reload_does_not_repeat_it() {
        let response = see_other("/somewhere");
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/somewhere"
        );
    }

    /// A message with a `&` or a space in it has to survive the round
    /// trip, or the page shows half an error.
    #[test]
    fn an_error_survives_the_redirect() {
        let response = back_with_error("/setup", "that is not the setup token & it expired");
        let location = response
            .headers()
            .get(header::LOCATION)
            .unwrap()
            .to_str()
            .unwrap();

        let query = location.split_once('?').expect("has a query").1;
        let decoded: HashMap<String, String> = form_urlencoded::parse(query.as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        assert_eq!(
            decoded.get("error").map(String::as_str),
            Some("that is not the setup token & it expired")
        );
    }

    #[test]
    fn a_signed_in_response_carries_the_cookie() {
        let response = see_other_with_cookie("/", sessions::set_cookie("t0ken"));
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("t0ken"));
        assert!(cookie.contains("HttpOnly"));
    }

    #[test]
    fn a_field_is_trimmed_and_missing_is_empty() {
        let mut form = HashMap::new();
        form.insert("username".to_string(), "  jorge  ".to_string());
        assert_eq!(field(&form, "username"), "jorge");
        assert_eq!(field(&form, "absent"), "");
    }
}
