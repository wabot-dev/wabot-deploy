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
use serde::Deserialize;
use wabot::prelude::*;
use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::request::Parts;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::{Middleware, RestResult};

use crate::domain::entities::account::Account;
use crate::infra::accounts::{self, sessions};

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
        // Which frontier this request arrived at, decided by the `Host`
        // before anything read a cookie — see `edge::Tenancy`.
        let scope = parts
            .extensions
            .get::<crate::infra::edge::Tenancy>()
            .map(|tenancy| tenancy.0.clone());

        match sessions::lookup(&self.state.database, &token).await {
            Ok(Some(account)) => {
                // **The identity and the frontier have to agree.** A
                // session is a valid session whichever hostname it is
                // presented at, so without this a tenant's cookie would
                // sign somebody in on the node's own console — narrowed
                // to nothing they could reach, but with the `Host` no
                // longer being the boundary. It is *also* stopped by the
                // cookie being host-only, and that is the browser's
                // courtesy rather than this node's rule.
                //
                // Nothing is assigned when they disagree, so the request
                // proceeds as a stranger and every page sends it to sign
                // in — the same place a wrong cookie leads.
                if !account.belongs_to(scope.as_deref()) {
                    tracing::debug!(
                        account = %account.id,
                        "a session for one scope was presented at another"
                    );
                    return Ok(());
                }
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

/// One query parameter, escaped.
///
/// Here rather than in a controller because both halves of the updates
/// page need it — the pages to send a refusal back, the forms to send
/// one on. Two copies of four lines is two places for the key to be
/// spelled differently.
pub fn encode(key: &str, value: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair(key, value)
        .finish()
}

/// Which capabilities a set of checkboxes agreed to.
///
/// **Presence, not value.** A checkbox that is off sends nothing at all,
/// so the whole set has to be drawn every time and absence is the
/// answer — a form built from part of the list once turned `store` off
/// in silence. Reading `Capability::ALL` is what makes a new one a
/// compile error rather than a field nobody wrote.
///
/// The prefix is the form's own spelling (`require-`, `offer-`, or none
/// at all), and it stops here: a use case that had to know it would be a
/// use case only a browser can call.
pub fn capabilities_ticked(
    form: &HashMap<String, String>,
    prefix: &str,
) -> Vec<crate::domain::values::capability::Capability> {
    crate::domain::values::capability::Capability::ALL
        .into_iter()
        .filter(|capability| form.contains_key(&format!("{prefix}{}", capability.name())))
        .collect()
}

pub fn source_from(
    form: &std::collections::HashMap<String, String>,
    name: &str,
) -> Result<crate::infra::edge::policy::RenewWith, String> {
    match field(form, "renew_with") {
        "self_signed" => Ok(crate::infra::edge::policy::RenewWith::SelfSigned),
        "file" => {
            let cert_path = field(form, "cert_path").trim().to_string();
            let key_path = field(form, "key_path").trim().to_string();
            if cert_path.is_empty() || key_path.is_empty() {
                return Err("reading from files needs both a certificate and a key".into());
            }
            // Refused now, while there is somebody to tell.
            crate::infra::edge::certs::from_files(name, &cert_path, &key_path)
                .map_err(|error| error.to_string())?;
            Ok(crate::infra::edge::policy::RenewWith::File {
                cert_path,
                key_path,
            })
        }
        // Including anything a hand-written form might send: ACME is the
        // default, and defaulting to it cannot lose a certificate the way
        // defaulting to a file source could.
        _ => Ok(crate::infra::edge::policy::RenewWith::Acme),
    }
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
/// The invitation link's own shape, shared by both consoles: the guest
/// door exists in each, because the invitation decides which frontier
/// somebody is born into and the address it carries is that frontier's.
#[derive(Debug, Deserialize, Validate)]
pub struct JoinPage {
    pub token: String,
    pub error: Option<String>,
}

/// Every page that can be redirected back to with an error takes one
/// of these.
#[derive(Debug, Deserialize, Validate)]
pub struct PageQuery {
    pub error: Option<String>,
}

/// Who this request is, if it is anybody.
pub fn signed_in(auth: &Auth) -> Option<Account> {
    auth.require_as::<Account>().ok()
}

fn text(status: StatusCode, body: &'static str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(body))
        .expect("a constant response is well-formed")
}

/// The project slug out of `/projects/{slug}/services…`.
pub fn project_slug(path: &str) -> Option<&str> {
    let segments = super::auth::segments(path);
    match segments.as_slice() {
        ["projects", slug, ..] if !slug.is_empty() => Some(slug),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slug_comes_out_of_the_path() {
        assert_eq!(project_slug("/projects/my-api/services"), Some("my-api"));
        assert_eq!(project_slug("/projects//services"), None);
        assert_eq!(project_slug("/elsewhere"), None);
    }
}
