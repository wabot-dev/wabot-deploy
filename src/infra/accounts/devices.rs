//! Browsers an account trusts. A passkey cannot say which device holds it,
//! because it syncs; a cookie this node set can (`docs/access.md`).

use wabot::rest::axum::http::HeaderMap;
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{sessions, sha256_hex, AccountResult};
use crate::infra::clock::now_ms;

pub const COOKIE_NAME: &str = "__Host-wabot_device";

const IDLE_MS: i64 = 90 * 86_400_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub account_id: String,
    pub name: String,
    pub passkey_id: Option<String>,
    pub created_at: i64,
    pub last_used_at: i64,
    pub last_address: Option<String>,
}

/// Trust this browser for this account, and the token its cookie holds.
/// A browser that already carries a token keeps it: another account may
/// trust it by the same one.
pub async fn trust(
    database: &SqliteDatabase,
    browser: Option<&str>,
    account_id: &str,
    name: &str,
    passkey_id: Option<&str>,
    address: Option<&str>,
) -> AccountResult<(String, String)> {
    let token = browser
        .filter(|token| !token.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| wabot::prelude::password::generate(48));
    let row = (
        format!("dev-{}", wabot::prelude::password::generate(16)),
        account_id.to_string(),
        sha256_hex(&token),
        name.to_string(),
        passkey_id.map(str::to_string),
        address.map(str::to_string),
    );
    let id = database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO trusted_device (\"id\", \"account_id\", \"token_hash\", \"name\", \
                 \"passkey_id\", \"created_at\", \"last_used_at\", \"last_address\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7) \
                 ON CONFLICT (\"token_hash\", \"account_id\") DO UPDATE SET \
                   \"name\" = excluded.\"name\", \"passkey_id\" = excluded.\"passkey_id\", \
                   \"last_used_at\" = excluded.\"last_used_at\", \
                   \"last_address\" = excluded.\"last_address\"",
                (&row.0, &row.1, &row.2, &row.3, &row.4, now_ms(), &row.5),
            )?;
            connection.query_row(
                "SELECT \"id\" FROM trusted_device WHERE \"token_hash\" = ?1 AND \"account_id\" = ?2",
                (&row.2, &row.1),
                |found| found.get::<_, String>(0),
            )
        })
        .await?;
    Ok((id, token))
}

/// The device this browser is to this account, if it trusts it still.
pub async fn recognise(
    database: &SqliteDatabase,
    browser: Option<&str>,
    account_id: &str,
    address: Option<&str>,
) -> AccountResult<Option<String>> {
    let Some(token) = browser.filter(|token| !token.is_empty()) else {
        return Ok(None);
    };
    let (hash, account, address) = (
        sha256_hex(token),
        account_id.to_string(),
        address.map(str::to_string),
    );
    Ok(database
        .write(move |connection| {
            let id: Option<String> = connection
                .query_row(
                    "SELECT \"id\" FROM trusted_device \
                     WHERE \"token_hash\" = ?1 AND \"account_id\" = ?2 AND \"last_used_at\" >= ?3",
                    (&hash, &account, now_ms() - IDLE_MS),
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(id) = &id {
                connection.execute(
                    "UPDATE trusted_device SET \"last_used_at\" = ?2, \
                     \"last_address\" = COALESCE(?3, \"last_address\") WHERE \"id\" = ?1",
                    (id, now_ms(), &address),
                )?;
            }
            Ok(id)
        })
        .await?)
}

/// What an account trusts. Trust lapses after ninety days unused, and a
/// lapsed device is forgotten here.
pub async fn of_account(database: &SqliteDatabase, account_id: &str) -> AccountResult<Vec<Device>> {
    let account = account_id.to_string();
    Ok(database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM trusted_device WHERE \"account_id\" = ?1 AND \"last_used_at\" < ?2",
                (&account, now_ms() - IDLE_MS),
            )?;
            connection
                .prepare(
                    "SELECT \"id\", \"account_id\", \"name\", \"passkey_id\", \"created_at\", \
                     \"last_used_at\", \"last_address\" FROM trusted_device \
                     WHERE \"account_id\" = ?1 ORDER BY \"last_used_at\" DESC",
                )?
                .query_map([&account], |row| {
                    Ok(Device {
                        id: row.get(0)?,
                        account_id: row.get(1)?,
                        name: row.get(2)?,
                        passkey_id: row.get(3)?,
                        created_at: row.get(4)?,
                        last_used_at: row.get(5)?,
                        last_address: row.get(6)?,
                    })
                })?
                .collect()
        })
        .await?)
}

/// Stop trusting one device, and end every session made on it. True when
/// it was theirs.
pub async fn withdraw(
    database: &SqliteDatabase,
    account_id: &str,
    device_id: &str,
) -> AccountResult<bool> {
    let (account, id) = (account_id.to_string(), device_id.to_string());
    let removed = database
        .write(move |connection| {
            Ok(connection.execute(
                "DELETE FROM trusted_device WHERE \"id\" = ?1 AND \"account_id\" = ?2",
                (id, account),
            )? > 0)
        })
        .await?;
    if removed {
        sessions::end_on_device(database, device_id).await?;
    }
    Ok(removed)
}

/// Stop trusting every device of an account.
pub async fn withdraw_all(database: &SqliteDatabase, account_id: &str) -> AccountResult<()> {
    let account = account_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM trusted_device WHERE \"account_id\" = ?1",
                [account],
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

pub fn from_headers(headers: &HeaderMap) -> Option<String> {
    sessions::cookie(headers, COOKIE_NAME)
}

/// Four hundred days, the longest a browser keeps a cookie: trust ends
/// on this node's terms, ninety days unused, not the browser's.
pub fn set_cookie(token: &str) -> String {
    format!("{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=34560000")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn someone() -> (SqliteDatabase, crate::domain::entities::account::Account) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let token = super::super::issue_setup_token(&database)
            .await
            .expect("token");
        let account = super::super::create_admin(&database, &token, "jorge", None)
            .await
            .expect("created");
        (database, account)
    }

    #[tokio::test]
    async fn a_browser_is_trusted_by_its_cookie_and_withdrawing_ends_its_sessions() {
        let (database, account) = someone().await;
        let (id, token) = trust(&database, None, &account.id, "Safari on macOS", None, None)
            .await
            .expect("trusted");

        assert_eq!(
            recognise(&database, Some(&token), &account.id, None)
                .await
                .expect("read"),
            Some(id.clone())
        );
        assert_eq!(
            recognise(&database, Some("another"), &account.id, None)
                .await
                .expect("read"),
            None
        );

        let session = sessions::start(&database, &account, Some(&id))
            .await
            .expect("session");
        assert!(withdraw(&database, &account.id, &id)
            .await
            .expect("withdrawn"));
        assert!(sessions::lookup(&database, &session)
            .await
            .expect("read")
            .is_none());
        assert!(recognise(&database, Some(&token), &account.id, None)
            .await
            .expect("read")
            .is_none());
    }

    #[tokio::test]
    async fn trust_lapses_after_ninety_days_unused() {
        let (database, account) = someone().await;
        let (_, token) = trust(&database, None, &account.id, "Firefox on Linux", None, None)
            .await
            .expect("trusted");
        database
            .write(|connection| {
                connection.execute("UPDATE trusted_device SET \"last_used_at\" = 1", [])?;
                Ok(())
            })
            .await
            .expect("aged");
        assert!(recognise(&database, Some(&token), &account.id, None)
            .await
            .expect("read")
            .is_none());
        assert!(of_account(&database, &account.id)
            .await
            .expect("read")
            .is_empty());
    }
}
