//! Browser sessions.
//!
//! A row per session, keyed by the hash of the cookie value. Storing
//! the value itself would mean a database anyone reads is a database
//! anyone logs in with — the same reasoning as passwords, for the same
//! reason.

use wabot::rest::axum::http::header::{HeaderMap, COOKIE};
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, sha256_hex, Account, AccountResult};

pub const COOKIE_NAME: &str = "wabot_session";

/// How long a session lasts without being used.
///
/// A week: long enough that an operator is not signing in every
/// morning, short enough that a laptop left in a café stops being a
/// way in.
const SESSION_DAYS: i64 = 7;

/// Start a session and return the cookie value.
///
/// The value exists in clear exactly once, here, on its way into a
/// `Set-Cookie`.
pub async fn create(database: &SqliteDatabase, account: &Account) -> AccountResult<String> {
    let token = wabot::prelude::password::generate(48);
    let (hash, id) = (sha256_hex(&token), account.id.clone());
    let expires_at = now_ms() + SESSION_DAYS * 86_400_000;

    database
        .write(move |connection| {
            // Expired rows are swept here rather than on a timer: this
            // is the only place that adds one.
            connection.execute("DELETE FROM session WHERE \"expires_at\" < ?1", [now_ms()])?;
            connection.execute(
                "INSERT INTO session (\"token_hash\", \"account_id\", \"created_at\", \"expires_at\") \
                 VALUES (?1, ?2, ?3, ?4)",
                (hash, id, now_ms(), expires_at),
            )?;
            Ok(())
        })
        .await?;

    Ok(token)
}

/// Whose session is this, if it is one?
pub async fn lookup(database: &SqliteDatabase, token: &str) -> AccountResult<Option<Account>> {
    let hash = sha256_hex(token);
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT a.\"id\", a.\"username\", a.\"role\", a.\"theme\", a.\"language\" \
                     FROM session s \
                     JOIN account a ON a.\"id\" = s.\"account_id\" \
                     WHERE s.\"token_hash\" = ?1 AND s.\"expires_at\" > ?2",
                    (hash, now_ms()),
                    |row| {
                        Ok(Account {
                            id: row.get(0)?,
                            username: row.get(1)?,
                            // Read on every request, so a role changed
                            // while somebody is signed in takes effect
                            // on their next click rather than at their
                            // next sign-in.
                            role: super::roles::NodeRole::parse(&row.get::<_, String>(2)?),
                            // Read here too, for the same reason: a
                            // theme chosen on one page has to be the
                            // one the next page renders in.
                            theme: crate::console::shell::Theme::parse(&row.get::<_, String>(3)?),
                            // And the language, for the third time the
                            // same reason: a page rendered in the other
                            // one is a console that forgot mid-click.
                            language: crate::console::language::Language::parse(
                                &row.get::<_, String>(4)?,
                            ),
                        })
                    },
                )
                .optional()
        })
        .await?)
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
///
/// Hand-parsed because pulling a cookie crate for one header is a
/// dependency for a `split`. Cookies are `a=1; b=2`, and a value can
/// contain `=` — so the split is on the *first* one only.
pub fn from_headers(headers: &HeaderMap) -> Option<String> {
    let header = headers.get(COOKIE)?.to_str().ok()?;
    header.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == COOKIE_NAME).then(|| value.to_string())
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
pub fn set_cookie(token: &str) -> String {
    format!(
        "{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
        SESSION_DAYS * 86_400
    )
}

/// The `Set-Cookie` that ends one.
pub fn clear_cookie() -> String {
    format!("{COOKIE_NAME}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn signed_in() -> (SqliteDatabase, Account, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let token = super::super::issue_setup_token(&database)
            .await
            .expect("token");
        let account =
            super::super::create_admin(&database, &token, "jorge", "correct horse battery")
                .await
                .expect("created");
        let session = create(&database, &account).await.expect("session");
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
        let cookie = set_cookie("t0ken");
        for attribute in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
            assert!(cookie.contains(attribute), "missing {attribute}: {cookie}");
        }
        assert!(clear_cookie().contains("Max-Age=0"));
    }
}
