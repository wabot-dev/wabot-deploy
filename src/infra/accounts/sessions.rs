//! Browser sessions.
//!
//! A row per session, keyed by the hash of the cookie value. Storing
//! the value itself would mean a database anyone reads is a database
//! anyone logs in with — the same reasoning as passwords, for the same
//! reason.

use wabot::rest::axum::http::header::{HeaderMap, COOKIE};
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{sha256_hex, Account, AccountResult};
use crate::domain::entities::session::Session;
use crate::infra::clock::now_ms;

/// `__Host-`, because a subdomain cannot set a cookie with that prefix:
/// a tenant's page under the node's domain could otherwise plant one the
/// console's host receives.
pub const COOKIE_NAME: &str = "__Host-wabot_session";

/// How long a session on a trusted device lasts.
///
/// A week: long enough that an operator is not signing in every
/// morning, short enough that a laptop left in a café stops being a
/// way in.
const SESSION_DAYS: i64 = 7;

/// How long a session on a device nobody trusted lasts, whatever the
/// browser does: one that restores its tabs restores session cookies too.
const UNTRUSTED_HOURS: i64 = 12;

/// Start a session and return the cookie value.
///
/// On a trusted device it lasts a week; on any other it ends with the
/// browser and, at the latest, twelve hours later. Starting one is proof
/// of who somebody is, so it counts as verified now.
///
/// The value exists in clear exactly once, here, on its way into a
/// `Set-Cookie`.
pub async fn start(
    database: &SqliteDatabase,
    account: &Account,
    device_id: Option<&str>,
) -> AccountResult<String> {
    let token = wabot::prelude::password::generate(48);
    let (hash, id) = (sha256_hex(&token), account.id.clone());
    let device = device_id.map(str::to_string);
    let now = now_ms();
    let expires_at = now
        + match device {
            Some(_) => SESSION_DAYS * 86_400_000,
            None => UNTRUSTED_HOURS * 3_600_000,
        };

    database
        .write(move |connection| {
            // Expired rows are swept here rather than on a timer: this
            // is the only place that adds one.
            connection.execute("DELETE FROM session WHERE \"expires_at\" < ?1", [now])?;
            connection.execute(
                "INSERT INTO session (\"token_hash\", \"account_id\", \"created_at\", \"expires_at\", \
                                      \"device_id\", \"verified_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?3)",
                (hash, id, now, expires_at, device),
            )?;
            Ok(())
        })
        .await?;

    Ok(token)
}

/// Whose session is this, if it is one?
#[cfg(test)]
pub async fn lookup(database: &SqliteDatabase, token: &str) -> AccountResult<Option<Account>> {
    Ok(find(database, token).await?.map(|(account, _)| account))
}

/// Whose session is this, and what is known about it.
pub async fn find(
    database: &SqliteDatabase,
    token: &str,
) -> AccountResult<Option<(Account, Session)>> {
    let hash = sha256_hex(token);
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT a.\"id\", a.\"username\", a.\"role\", a.\"theme\", a.\"language\", \
                     a.\"tenant_id\", s.\"token_hash\", s.\"device_id\", s.\"verified_at\" \
                     FROM session s \
                     JOIN account a ON a.\"id\" = s.\"account_id\" \
                     WHERE s.\"token_hash\" = ?1 AND s.\"expires_at\" > ?2",
                    (hash, now_ms()),
                    |row| {
                        Ok((
                            Account {
                                id: row.get(0)?,
                                username: row.get(1)?,
                                // Read on every request, so a role changed
                                // while somebody is signed in takes effect
                                // on their next click rather than at their
                                // next sign-in.
                                role: crate::domain::values::roles::NodeRole::parse(
                                    &row.get::<_, String>(2)?,
                                ),
                                theme: crate::domain::values::theme::Theme::parse(
                                    &row.get::<_, String>(3)?,
                                ),
                                language: crate::domain::values::language::Language::parse(
                                    &row.get::<_, String>(4)?,
                                ),
                                // What the `Host` is checked against, so it
                                // has to be the row's answer now rather than
                                // the one it gave when they signed in.
                                tenant_id: row.get(5)?,
                            },
                            Session {
                                token_hash: row.get(6)?,
                                device_id: row.get(7)?,
                                verified_at: row.get(8)?,
                            },
                        ))
                    },
                )
                .optional()
        })
        .await?)
}

/// They proved who they are again.
pub async fn mark_verified(database: &SqliteDatabase, token_hash: &str) -> AccountResult<()> {
    let hash = token_hash.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE session SET \"verified_at\" = ?2 WHERE \"token_hash\" = ?1",
                (hash, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The session was made on a device that is trusted now, so it lasts as
/// a trusted device's does.
pub async fn bind(
    database: &SqliteDatabase,
    token_hash: &str,
    device_id: &str,
) -> AccountResult<()> {
    let (hash, device) = (token_hash.to_string(), device_id.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE session SET \"device_id\" = ?2, \"expires_at\" = ?3 WHERE \"token_hash\" = ?1",
                (hash, device, now_ms() + SESSION_DAYS * 86_400_000),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// End every session made on one device.
pub async fn end_on_device(database: &SqliteDatabase, device_id: &str) -> AccountResult<()> {
    let device = device_id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM session WHERE \"device_id\" = ?1", [device])?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// End every session on the node. What a restore does, because the
/// backup's rows may make a revoked cookie valid again.
pub async fn end_everyone(database: &SqliteDatabase) -> AccountResult<()> {
    database
        .write(|connection| {
            connection.execute("DELETE FROM session", [])?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// End every session of one account.
pub async fn end_all(database: &SqliteDatabase, account_id: &str) -> AccountResult<()> {
    let account = account_id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM session WHERE \"account_id\" = ?1", [account])?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// End one session.
pub async fn revoke(database: &SqliteDatabase, token: &str) -> AccountResult<()> {
    let hash = sha256_hex(token);
    database
        .write(move |connection| {
            connection.execute("DELETE FROM session WHERE \"token_hash\" = ?1", [hash])?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The session cookie's value out of a request's headers.
pub fn from_headers(headers: &HeaderMap) -> Option<String> {
    cookie(headers, COOKIE_NAME)
}

/// A cookie's value out of a request's headers.
///
/// Hand-parsed because pulling a cookie crate for one header is a
/// dependency for a `split`. Cookies are `a=1; b=2`, and a value can
/// contain `=` — so the split is on the *first* one only.
///
/// Every `cookie` field is read, not the first: over HTTP/2 a browser may
/// send one field per cookie, and the console is served over HTTP/2.
pub fn cookie(headers: &HeaderMap, wanted: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|field| field.to_str().ok())
        .flat_map(|field| field.split(';'))
        .find_map(|pair| {
            let (name, value) = pair.trim().split_once('=')?;
            (name == wanted && !value.is_empty()).then(|| value.to_string())
        })
}

/// The `Set-Cookie` that starts a session.
///
/// Every attribute here is load-bearing:
///
/// * `HttpOnly` — script cannot read it, so an XSS is not a stolen
///   session.
/// * `Secure` — never sent over plain HTTP. The node redirects :80 to
///   :443 anyway, and this is what stops a downgrade from leaking it.
/// * `SameSite=Lax` — not sent on a cross-site POST, which is CSRF
///   protection for every mutating form on the console.
/// * `Path=/` — the whole console, not a subtree.
///
/// Only a session on a trusted device outlives the browser.
pub fn set_cookie(token: &str, lasting: bool) -> String {
    let base = format!("{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Lax; Path=/");
    match lasting {
        true => format!("{base}; Max-Age={}", SESSION_DAYS * 86_400),
        false => base,
    }
}

/// The `Set-Cookie` that ends one.
pub fn clear_cookie() -> String {
    format!("{COOKIE_NAME}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn signed_in() -> (SqliteDatabase, Account, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let token = super::super::issue_setup_token(&database)
            .await
            .expect("token");
        let account = super::super::create_admin(&database, &token, "jorge", None)
            .await
            .expect("created");
        let session = start(&database, &account, None).await.expect("session");
        (database, account, session)
    }

    #[tokio::test]
    async fn a_session_round_trips() {
        let (database, account, session) = signed_in().await;
        let found = lookup(&database, &session)
            .await
            .expect("query")
            .expect("present");
        assert_eq!(found.id, account.id);
    }

    #[tokio::test]
    async fn an_unknown_token_is_nobody() {
        let (database, _, _) = signed_in().await;
        assert!(lookup(&database, "made-up").await.expect("query").is_none());
    }

    #[tokio::test]
    async fn revoking_ends_it() {
        let (database, _, session) = signed_in().await;
        revoke(&database, &session).await.expect("revoke");
        assert!(lookup(&database, &session).await.expect("query").is_none());
    }

    #[tokio::test]
    async fn an_expired_session_is_nobody() {
        let (database, _, session) = signed_in().await;
        let hash = sha256_hex(&session);
        database
            .write(move |connection| {
                connection.execute(
                    "UPDATE session SET \"expires_at\" = 1 WHERE \"token_hash\" = ?1",
                    [hash],
                )?;
                Ok(())
            })
            .await
            .expect("expire");

        assert!(lookup(&database, &session).await.expect("query").is_none());
    }

    /// A database someone reads must not be a database someone signs
    /// in with.
    #[tokio::test]
    async fn the_cookie_value_is_not_stored() {
        let (database, _, session) = signed_in().await;
        let stored: String = database
            .read(|connection| {
                connection.query_row("SELECT \"token_hash\" FROM session", [], |row| row.get(0))
            })
            .await
            .expect("query");
        assert_ne!(stored, session);
        assert_eq!(stored, sha256_hex(&session));
    }

    #[test]
    fn the_cookie_is_found_among_others() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            format!("theme=dark; {COOKIE_NAME}=abc123; other=1")
                .parse()
                .unwrap(),
        );
        assert_eq!(from_headers(&headers), Some("abc123".to_string()));

        let mut absent = HeaderMap::new();
        absent.insert(COOKIE, "theme=dark".parse().unwrap());
        assert_eq!(from_headers(&absent), None);
        assert_eq!(from_headers(&HeaderMap::new()), None);
    }

    /// Safari over HTTP/2 sends each cookie in a field of its own, and
    /// the session was only looked for in the first. The edge proxy had
    /// the same fault and it broke every form on Nubo; here it waits for a
    /// second cookie on the console's host — one set with `Domain=` by a
    /// sibling site is enough — and then signs the operator out on every
    /// request.
    #[test]
    fn the_cookie_is_found_in_any_of_several_fields() {
        let mut headers = HeaderMap::new();
        headers.append(COOKIE, "_ga=GA1.1".parse().unwrap());
        headers.append(COOKIE, format!("{COOKIE_NAME}=abc123").parse().unwrap());
        assert_eq!(from_headers(&headers), Some("abc123".to_string()));
    }

    /// A base64-ish token can contain `=`, so splitting on every one
    /// would truncate the session and log the operator out with no
    /// explanation.
    #[test]
    fn a_value_containing_equals_survives() {
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, format!("{COOKIE_NAME}=ab=cd=").parse().unwrap());
        assert_eq!(from_headers(&headers), Some("ab=cd=".to_string()));
    }

    /// Each attribute stops a different attack, and dropping one is
    /// silent — the cookie still works.
    #[test]
    fn the_cookie_carries_every_protection() {
        for lasting in [true, false] {
            let cookie = set_cookie("t0ken", lasting);
            for attribute in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
                assert!(cookie.contains(attribute), "missing {attribute}: {cookie}");
            }
        }
        assert!(clear_cookie().contains("Max-Age=0"));
    }

    /// A session on a device nobody trusted is one a browser forgets when
    /// it closes, and the row ends twelve hours on whatever the browser
    /// does.
    ///
    /// It failed once in the v0.46.2 release run, a millisecond short:
    /// `start` read the clock for the expiry and again for the row, and a
    /// tick between them shortened every session by it.
    #[tokio::test]
    async fn only_a_trusted_device_keeps_a_session_past_the_browser() {
        assert!(set_cookie("t", true).contains("Max-Age="));
        assert!(!set_cookie("t", false).contains("Max-Age="));

        let (database, account, untrusted) = signed_in().await;
        let trusted = start(&database, &account, Some("dev-1"))
            .await
            .expect("one");
        let expiry = |token: String| {
            let database = database.clone();
            async move {
                let hash = sha256_hex(&token);
                database
                    .read(move |connection| {
                        connection.query_row(
                            "SELECT \"expires_at\" - \"created_at\" FROM session \
                             WHERE \"token_hash\" = ?1",
                            [hash],
                            |row| row.get::<_, i64>(0),
                        )
                    })
                    .await
                    .expect("a row")
            }
        };
        assert_eq!(expiry(untrusted).await, UNTRUSTED_HOURS * 3_600_000);
        assert_eq!(expiry(trusted.clone()).await, SESSION_DAYS * 86_400_000);

        end_on_device(&database, "dev-1").await.expect("ended");
        assert!(lookup(&database, &trusted).await.expect("query").is_none());
    }
}
