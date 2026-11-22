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
//! is the origin check on both routers (`refuse_cross_origin`) with
//! `SameSite=Lax` behind it, rather than a token to thread through every
//! form: `SameSite` alone lets a page under the node's own domain in.
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
use crate::domain::entities::session::Session;
use crate::domain::values::browser::{Browser, Ceremony};
use crate::infra::accounts::{self, sessions};

use crate::shell::state::ConsoleState;

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
        // Which frontier this request arrived at, decided by the `Host`
        // before anything read a cookie — see `edge::Tenancy`.
        let scope = parts
            .extensions
            .get::<crate::infra::edge::Tenancy>()
            .map(|tenancy| tenancy.0.clone());
        container.register_instance::<Asked>(Arc::new(Asked {
            host: host_of(&parts.headers)
                .or_else(|| parts.uri.authority().map(|authority| authority.to_string()))
                .unwrap_or_default(),
            scope: scope.clone(),
            pending: accounts::requests::from_headers(&parts.headers),
        }));
        let Some(token) = sessions::from_headers(&parts.headers) else {
            return Ok(());
        };

        match sessions::find(&self.state.database, &token).await {
            Ok(Some((account, session))) => {
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
                let _ = auth.assign(Signed { account, session });
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

/// What a page needs to know about the request it renders, which a view
/// is not handed: the host a ceremony is bound to, the frontier, and the
/// cookie a sign-in page reads. Registered per request by the
/// middleware; the root holds an empty one for a view without it.
#[derive(Debug, Clone, Default)]
pub struct Asked {
    pub host: String,
    pub scope: Option<String>,
    pub pending: Option<String>,
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

/// A path a form says to go back to, if it is one of this console's.
///
/// `//host/` and `/\host/` start with a slash and are another site:
/// a browser reads both as a scheme-relative URL. The theme and language
/// switches checked only the slash, which made `from=//evil.example/` a
/// redirect anywhere — found on a VM node.
pub fn local_path(from: &str) -> Option<&str> {
    let mut chars = from.chars();
    match (chars.next(), chars.next()) {
        (Some('/'), Some('/' | '\\')) => None,
        (Some('/'), _) => Some(from),
        _ => None,
    }
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

/// One name per box rather than one name for all of them: the form is
/// read into a map, which keeps one value per name.
pub fn ticked(form: &HashMap<String, String>, prefix: &str) -> Vec<String> {
    let mut ticked: Vec<String> = form
        .keys()
        .filter_map(|name| name.strip_prefix(prefix))
        .filter(|rest| !rest.is_empty())
        .map(str::to_string)
        .collect();
    ticked.sort();
    ticked
}

/// `KEY=value` lines into pairs.
///
/// The split is on the *first* `=` only: a value is very often a URL or
/// a base64 blob with one in it, and splitting on every `=` truncates
/// exactly the secrets somebody would least like truncated.
///
/// Blank lines and `#` comments are skipped, so a block pasted out of
/// a `.env` file works as pasted.
pub fn parse_env(text: &str) -> Result<Vec<(String, String)>, String> {
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

pub fn asks_no_authority(form: &std::collections::HashMap<String, String>) -> bool {
    matches!(field(form, "renew_with"), "self_signed" | "file")
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

/// A location, with the refusal a page was carrying when it has one.
///
/// For a page that moved: a link kept from before lands on the thing it
/// named, and what it was about to say arrives with it.
pub fn with_error(location: &str, error: Option<&str>) -> String {
    match error {
        Some(message) => format!("{location}?{}", encode("error", message)),
        None => location.to_string(),
    }
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

/// What `Auth` holds: the account every page reads, and the session the
/// few that make a way in also need.
#[derive(serde::Serialize, Deserialize)]
struct Signed {
    #[serde(flatten)]
    account: Account,
    session: Session,
}

pub fn session(auth: &Auth) -> Option<Session> {
    auth.require_as::<Signed>()
        .ok()
        .map(|signed| signed.session)
}

/// Who is at the door. Read before the body, which the form consumes.
pub fn browser(request: &Request) -> Browser {
    let headers = request.headers();
    let host = host_of(headers)
        .or_else(|| {
            request
                .uri()
                .authority()
                .map(|authority| authority.to_string())
        })
        .unwrap_or_default();
    let agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    Browser {
        host,
        device: accounts::devices::from_headers(headers),
        name: crate::domain::values::device_name::from_user_agent(agent),
        address: crate::infra::edge::client_address(request).map(|address| address.to_string()),
    }
}

fn host_of(headers: &wabot::rest::axum::http::HeaderMap) -> Option<String> {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string)
}

/// A passkey's answer out of a form: `challenge` and `credential`, or the
/// same with a prefix.
pub fn ceremony(form: &HashMap<String, String>, prefix: &str) -> Ceremony {
    Ceremony {
        challenge: field(form, &format!("{prefix}challenge")).to_string(),
        credential: field(form, &format!("{prefix}credential")).to_string(),
    }
}

/// **Asked again before anything that makes a way in.** A stolen session
/// cookie must not become permanent access: a passkey verified in the last
/// five minutes, or one given with this form.
pub async fn proven(
    state: &ConsoleState,
    auth: &Auth,
    browser: &Browser,
    form: &HashMap<String, String>,
) -> Result<(), String> {
    let (Some(account), Some(session)) = (signed_in(auth), session(auth)) else {
        return Err(accounts::ceremonies::ASK_AGAIN.into());
    };
    accounts::ceremonies::asked_again(
        &state.database,
        &browser.host,
        &session,
        &account.id,
        &ceremony(form, "verify_"),
    )
    .await
}

/// The prompt a page renders before a form that makes a way in: none when
/// the session is fresh or the account has no passkey here to give.
pub async fn verify_prompt(
    state: &ConsoleState,
    auth: &Auth,
    host: &str,
) -> Option<accounts::ceremonies::Begun> {
    let (account, session) = (signed_in(auth)?, session(auth)?);
    if session.fresh(crate::infra::clock::now_ms()) {
        return None;
    }
    let rp = accounts::ceremonies::relying_party(host)?;
    if accounts::passkeys::ids_for(&state.database, &account.id, &rp.id)
        .await
        .ok()?
        .is_empty()
    {
        return None;
    }
    accounts::ceremonies::begin_assertion(
        &state.database,
        &rp,
        accounts::challenges::Purpose::Verify,
        Some(&account.id),
    )
    .await
    .ok()
}

/// A 303 setting several cookies.
pub fn see_other_with_cookies(location: &str, cookies: &[String]) -> Response {
    let mut response = Response::builder()
        .status(StatusCode::SEE_OTHER)
        .header(header::LOCATION, location);
    for cookie in cookies {
        response = response.header(header::SET_COOKIE, cookie);
    }
    response
        .body(Body::empty())
        .expect("a constant response is well-formed")
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
