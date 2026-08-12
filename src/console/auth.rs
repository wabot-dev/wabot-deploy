//! Who is asking, and what happens when nobody is.
//!
//! ## The middleware annotates; it does not reject
//!
//! The framework's `Middleware` is rejection-only, and a rejection is
//! a JSON error body. For a browser that is the wrong answer — a
//! visitor with no session wants the sign-in page, not a 401 with
//! `{"error":…}` in it.
//!
//! So this middleware only ever succeeds: it reads the cookie, and if
//! it names a live session it assigns the account to [`Auth`]. Each
//! view then decides, and a view that needs somebody returns a
//! redirect, which `ViewOutcome` can express and a `RestError` cannot.
//!
//! ## Plain forms, no JavaScript
//!
//! A POST and a 303. It works with scripting off, and the CSRF story
//! is `SameSite=Lax` on the session cookie rather than a token to
//! thread through every form.
//!
//! POSTs are `#[raw]` REST endpoints rather than `#[action]`s: an
//! action answers JSON for the boosted-nav client, and what a form
//! needs back is a 303 carrying `Set-Cookie`.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::request::Parts;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::{Middleware, RestResult};
use wabot::ui::hypertext::IntoView;

use super::language::t;
use crate::accounts::{self, sessions, Account};

use super::layout;
use super::ConsoleState;

/// Reads the session cookie and records who it names.
///
/// Never refuses. A visitor with no session reaches the view with an
/// unassigned `Auth`, and the view sends them somewhere useful.
#[singleton]
pub struct SessionMiddleware {
    state: Arc<ConsoleState>,
}

#[async_trait]
impl Middleware for SessionMiddleware {
    async fn handle(&self, parts: &Parts, container: &Container) -> RestResult<()> {
        let Some(token) = sessions::from_headers(&parts.headers) else {
            return Ok(());
        };
        match sessions::lookup(&self.state.database, &token).await {
            Ok(Some(account)) => {
                let auth = Auth::of(container);
                let _ = auth.assign(account);
            }
            Ok(None) => {}
            // A database that cannot answer is not a reason to refuse
            // the request: the view will send them to sign in, which
            // is the same place a wrong cookie leads.
            Err(error) => tracing::warn!(%error, "could not read the session"),
        }
        Ok(())
    }
}

/// Where an unauthenticated visitor belongs.
///
/// Setup when the node has nobody yet, sign-in when it does. A
/// redirect rather than a rendered page, so every protected URL
/// behaves the same way signed out.
pub async fn signed_out_destination(state: &ConsoleState) -> &'static str {
    match accounts::any_account(&state.database).await {
        Ok(true) => "/sign-in",
        // Nobody yet — or the database is unreadable, and setup is
        // where an operator can act on that.
        _ => "/setup",
    }
}

// ---------- form bodies --------------------------------------------------

/// Read an `application/x-www-form-urlencoded` body.
///
/// Capped: a form is a few hundred bytes, and an unbounded read is a
/// way to make the node hold a gigabyte for one request.
pub async fn read_form(request: Request) -> Result<HashMap<String, String>, Response> {
    const MAX: usize = 64 * 1024;

    let Ok(bytes) = wabot::rest::axum::body::to_bytes(request.into_body(), MAX).await else {
        return Err(text(StatusCode::BAD_REQUEST, "that form is too large"));
    };
    Ok(form_urlencoded::parse(&bytes)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect())
}

/// A field, trimmed, or the empty string.
pub fn field<'a>(form: &'a HashMap<String, String>, name: &str) -> &'a str {
    form.get(name).map(|value| value.trim()).unwrap_or_default()
}

// ---------- the responses a form POST needs ------------------------------

/// 303 See Other.
///
/// Not 302: after a POST this tells the browser to follow up with a
/// GET, which is what keeps the result off the reload button.
pub fn see_other(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

/// 303, and a session cookie with it.
pub fn see_other_with_cookie(location: &str, cookie: String) -> Response {
    Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location)
        .header(header::SET_COOKIE, cookie)
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

/// A form that was refused, carried back to the page that submitted it.
///
/// In a query parameter rather than a session flash: one fewer thing
/// with a lifetime, and a refused form is not secret. The page
/// re-renders with the message and the operator fixes it.
pub fn back_with_error(location: &str, message: &str) -> Response {
    let encoded: String = form_urlencoded::Serializer::new(String::new())
        .append_pair("error", message)
        .finish();
    see_other(&format!("{location}?{encoded}"))
}

/// A path split into its parts.
///
/// `#[raw]` endpoints get the whole request and no extracted path
/// parameters — the trade for being able to answer with a 303 — so the
/// slugs come out of the URI here.
///
/// Only the leading and trailing slashes are dropped. An *interior*
/// empty segment is kept, because discarding it slides every later
/// part one place left: `/projects//services` would otherwise read as
/// the project named "services".
pub fn segments(path: &str) -> Vec<&str> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    trimmed.split('/').collect()
}

/// A query string carrying nothing but a refused form's message.
///
/// Every page that can be redirected back to with an error takes one
/// of these.
#[derive(Debug, Deserialize, Validate)]
pub struct PageQuery {
    pub error: Option<String>,
    /// An invitation link, shown once by the page that just made it.
    /// Carried in the query rather than stored: the token exists in
    /// clear exactly once, and a page that could show it again would
    /// mean it was kept somewhere.
    pub invited: Option<String>,
}

/// Who this request is, if it is anybody.
pub fn signed_in(auth: &Auth) -> Option<Account> {
    auth.require_as::<Account>().ok()
}

// ---------- pages --------------------------------------------------------

/// Setup and sign-in. The only two pages a stranger may see.
#[injectable]
pub struct AuthPages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl AuthPages {
    /// Create the first administrator.
    ///
    /// Guarded twice over: the page is gone once an account exists, and
    /// the POST re-checks. The page going away is courtesy; the POST
    /// check is the security boundary, because a page is only a page.
    #[view("/setup")]
    #[middleware(SessionMiddleware)]
    async fn setup(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        if accounts::any_account(&self.state.database).await? {
            return Ok(Redirect::found("/sign-in").into());
        }
        // Without a token there is nothing to type, and saying so beats
        // a form that can only be refused.
        let has_token = accounts::setup_token_valid(&self.state.database).await?;

        layout::head("Set up");
        Ok(rsx! {
            (layout::style_tag())
            <main class="shell narrow">
                (mark("Set up this node"))
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                @if has_token {
                    <p class="tagline">(t("The setup token was printed by `wabot-deploy install`. \
                         It works once, and it expires."))</p>
                    <form method="post" action="/setup" class="card stack">
                        <label for="setup_token">(t("Setup token"))</label>
                        <input id="setup_token" name="setup_token" type="text"
                               autocomplete="off" required>

                        <label for="username">(t("Username"))</label>
                        <input id="username" name="username" type="text"
                               autocomplete="username" required>

                        <label for="password">(t("Password"))</label>
                        <input id="password" name="password" type="password"
                               autocomplete="new-password" required>
                        <p class="field-hint">(t("At least 12 characters. A phrase beats a puzzle — \
                             this console can start containers on the machine."))</p>

                        <div class="actions">
                            <button type="submit">(t("Create administrator"))</button>
                        </div>
                    </form>
                } @else {
                    <section class="card stack">
                        <p>(t("No setup token is outstanding, so nobody can be created \
                             from here. Issue one on the node:"))</p>
                        <pre><code>("wabot-deploy setup-token")</code></pre>
                    </section>
                }
            </main>
        }
        .into_view()
        .into())
    }

    #[view("/sign-in")]
    #[middleware(SessionMiddleware)]
    async fn sign_in(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        if signed_in(&self.auth).is_some() {
            return Ok(Redirect::found("/").into());
        }
        // A node with nobody on it cannot be signed into, and sending
        // somebody to a form that will always refuse them is a worse
        // answer than sending them to the one that works.
        if !accounts::any_account(&self.state.database).await? {
            return Ok(Redirect::found("/setup").into());
        }

        layout::head("Sign in");
        Ok(rsx! {
            (layout::style_tag())
            <main class="shell narrow">
                (mark("Sign in"))
                @if let Some(message) = &query.error {
                    (layout::error_note(message))
                }
                <form method="post" action="/sign-in" class="card stack">
                    <label for="username">(t("Username"))</label>
                    <input id="username" name="username" type="text"
                           autocomplete="username" required>

                    <label for="password">(t("Password"))</label>
                    <input id="password" name="password" type="password"
                           autocomplete="current-password" required>

                    <div class="actions">
                        <button type="submit">(t("Sign in"))</button>
                    </div>
                </form>
            </main>
        }
        .into_view()
        .into())
    }
}

/// The logo and a heading — what a page with no header bar opens with.
pub(crate) fn mark(heading: &str) -> impl Renderable + '_ {
    rsx! {
        <header class="mark">
            <img
                src=(format!("{}/wabot-logo.png", super::assets::MOUNT))
                alt="Wabot" width="36" height="36">
            <div class="stack-sm">
                <h1>(heading)</h1>
                <p class="tagline">("wabot-deploy")</p>
            </div>
        </header>
    }
}

// ---------- form submissions ---------------------------------------------

/// The POSTs behind those two pages, and sign-out.
///
/// `#[raw]` throughout: what a form needs back is a 303 with a
/// `Set-Cookie` on it, and the JSON path cannot express either.
#[injectable]
pub struct AuthApi {
    state: Arc<ConsoleState>,
    /// Only `/theme` needs it — it writes against whoever is asking.
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl AuthApi {
    #[post("/setup")]
    #[raw]
    async fn create_admin(&self, request: Request) -> RestResult<Response> {
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let account = match accounts::create_admin(
            &self.state.database,
            field(&form, "setup_token"),
            field(&form, "username"),
            // Not trimmed: leading and trailing spaces are part of a
            // password, and silently removing them here would mean a
            // password that cannot be typed back.
            form.get("password").map(String::as_str).unwrap_or_default(),
        )
        .await
        {
            Ok(account) => account,
            Err(error) => return Ok(back_with_error("/setup", &error.to_string())),
        };

        // Signed in immediately. Creating the account and then being
        // asked to prove it is the same secret twice in a row.
        match sessions::create(&self.state.database, &account).await {
            Ok(token) => Ok(see_other_with_cookie("/", sessions::set_cookie(&token))),
            // The account exists; only the session failed. Sending them
            // to sign in is honest and recoverable.
            Err(error) => {
                tracing::error!(%error, "could not start a session after setup");
                Ok(see_other("/sign-in"))
            }
        }
    }

    #[post("/sign-in")]
    #[raw]
    async fn sign_in(&self, request: Request) -> RestResult<Response> {
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let account = accounts::authenticate(
            &self.state.database,
            field(&form, "username"),
            form.get("password").map(String::as_str).unwrap_or_default(),
        )
        .await?;

        let Some(account) = account else {
            // One message for a wrong username and a wrong password.
            // Telling them apart tells an attacker which usernames
            // exist, which is the whole point of the dummy hash in
            // `authenticate`.
            return Ok(back_with_error(
                "/sign-in",
                "that username and password do not match",
            ));
        };

        let token = sessions::create(&self.state.database, &account).await?;
        Ok(see_other_with_cookie("/", sessions::set_cookie(&token)))
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

        let theme = super::shell::Theme::parse(field(&form, "theme"));
        if let Err(error) =
            crate::accounts::set_theme(&self.state.database, &account.id, theme).await
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

        let language = super::language::Language::parse(field(&form, "language"));
        if let Err(error) =
            crate::accounts::set_language(&self.state.database, &account.id, language).await
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

fn text(status: StatusCode, body: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(body))
        .expect("a constant response is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;

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
