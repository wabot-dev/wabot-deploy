//! A new device asking to be let in, after its passkey passed, and the
//! code a trusted device gives it (`docs/access.md`, "Signing in").

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{constant_time_eq, sessions, sha256_hex, AccountResult};
use crate::infra::clock::now_ms;

pub const COOKIE_NAME: &str = "__Host-wabot_pending";

const LIFETIME_MS: i64 = 5 * 60_000;
const ATTEMPTS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRequest {
    pub id: String,
    pub account_id: String,
    pub passkey_id: String,
    pub name: String,
    pub address: Option<String>,
    pub approved: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redeemed {
    Accepted(DeviceRequest),
    NotYet,
    Wrong,
    Gone,
}

const COLUMNS: &str = "\"id\", \"account_id\", \"passkey_id\", \"name\", \"address\", \
                       \"code_hash\" IS NOT NULL, \"created_at\"";

fn decode(
    row: &wabot::sqlite::rusqlite::Row<'_>,
) -> wabot::sqlite::rusqlite::Result<DeviceRequest> {
    Ok(DeviceRequest {
        id: row.get(0)?,
        account_id: row.get(1)?,
        passkey_id: row.get(2)?,
        name: row.get(3)?,
        address: row.get(4)?,
        approved: row.get(5)?,
        created_at: row.get(6)?,
    })
}

/// Open one, replacing whatever the account had pending: one at a time is
/// what keeps a trusted device from being asked about a crowd.
pub async fn open(
    database: &SqliteDatabase,
    account_id: &str,
    passkey_id: &str,
    name: &str,
    address: Option<&str>,
) -> AccountResult<String> {
    let token = wabot::prelude::password::generate(48);
    let row = (
        format!("req-{}", wabot::prelude::password::generate(16)),
        account_id.to_string(),
        sha256_hex(&token),
        passkey_id.to_string(),
        name.to_string(),
        address.map(str::to_string),
    );
    database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM device_request WHERE \"account_id\" = ?1 OR \"expires_at\" < ?2",
                (&row.1, now_ms()),
            )?;
            connection.execute(
                "INSERT INTO device_request (\"id\", \"account_id\", \"browser_hash\", \
                 \"passkey_id\", \"name\", \"address\", \"created_at\", \"expires_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                (
                    row.0,
                    row.1,
                    row.2,
                    row.3,
                    row.4,
                    row.5,
                    now_ms(),
                    now_ms() + LIFETIME_MS,
                ),
            )?;
            Ok(())
        })
        .await?;
    Ok(token)
}

pub async fn pending_for(
    database: &SqliteDatabase,
    account_id: &str,
) -> AccountResult<Option<DeviceRequest>> {
    let account = account_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM device_request \
                         WHERE \"account_id\" = ?1 AND \"expires_at\" >= ?2"
                    ),
                    (account, now_ms()),
                    decode,
                )
                .optional()
        })
        .await?)
}

pub async fn by_browser(
    database: &SqliteDatabase,
    token: &str,
) -> AccountResult<Option<DeviceRequest>> {
    let hash = sha256_hex(token);
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM device_request \
                         WHERE \"browser_hash\" = ?1 AND \"expires_at\" >= ?2"
                    ),
                    (hash, now_ms()),
                    decode,
                )
                .optional()
        })
        .await?)
}

/// Approve one of an account's requests, and the six digits its browser
/// has to type. `None` when there is no such request any more.
pub async fn approve(
    database: &SqliteDatabase,
    account_id: &str,
    id: &str,
) -> AccountResult<Option<String>> {
    let code = wabot::prelude::random::digits(6);
    let (account, id, hash) = (account_id.to_string(), id.to_string(), sha256_hex(&code));
    let changed = database
        .write(move |connection| {
            connection.execute(
                "UPDATE device_request SET \"code_hash\" = ?3, \"attempts\" = 0 \
                 WHERE \"id\" = ?1 AND \"account_id\" = ?2 AND \"expires_at\" >= ?4",
                (id, account, hash, now_ms()),
            )
        })
        .await?;
    Ok((changed > 0).then_some(code))
}

/// Refuse one of an account's requests, and what it was.
pub async fn refuse(
    database: &SqliteDatabase,
    account_id: &str,
    id: &str,
) -> AccountResult<Option<DeviceRequest>> {
    let (account, id) = (account_id.to_string(), id.to_string());
    Ok(database
        .write(move |connection| {
            let found = connection
                .query_row(
                    &format!(
                        "SELECT {COLUMNS} FROM device_request \
                         WHERE \"id\" = ?1 AND \"account_id\" = ?2"
                    ),
                    (&id, &account),
                    decode,
                )
                .optional()?;
            connection.execute("DELETE FROM device_request WHERE \"id\" = ?1", [&id])?;
            Ok(found)
        })
        .await?)
}

/// The browser that opened a request types the code it was given.
pub async fn redeem(database: &SqliteDatabase, token: &str, code: &str) -> AccountResult<Redeemed> {
    let (hash, typed) = (sha256_hex(token), sha256_hex(code.trim()));
    Ok(database
        .write(move |connection| {
            let found = connection
                .query_row(
                    &format!(
                        "SELECT {COLUMNS}, \"code_hash\", \"attempts\" FROM device_request \
                         WHERE \"browser_hash\" = ?1 AND \"expires_at\" >= ?2"
                    ),
                    (&hash, now_ms()),
                    |row| {
                        Ok((
                            decode(row)?,
                            row.get::<_, Option<String>>(7)?,
                            row.get::<_, i64>(8)?,
                        ))
                    },
                )
                .optional()?;
            let Some((request, code_hash, attempts)) = found else {
                return Ok(Redeemed::Gone);
            };
            let Some(code_hash) = code_hash else {
                return Ok(Redeemed::NotYet);
            };
            if constant_time_eq(&code_hash, &typed) {
                connection.execute(
                    "DELETE FROM device_request WHERE \"id\" = ?1",
                    [&request.id],
                )?;
                return Ok(Redeemed::Accepted(request));
            }
            if attempts + 1 >= ATTEMPTS {
                connection.execute(
                    "DELETE FROM device_request WHERE \"id\" = ?1",
                    [&request.id],
                )?;
                return Ok(Redeemed::Gone);
            }
            connection.execute(
                "UPDATE device_request SET \"attempts\" = \"attempts\" + 1 WHERE \"id\" = ?1",
                [&request.id],
            )?;
            Ok(Redeemed::Wrong)
        })
        .await?)
}

pub fn from_headers(headers: &wabot::rest::axum::http::HeaderMap) -> Option<String> {
    sessions::cookie(headers, COOKIE_NAME)
}

pub fn set_cookie(token: &str) -> String {
    format!(
        "{COOKIE_NAME}={token}; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age={}",
        LIFETIME_MS / 1000
    )
}

pub fn clear_cookie() -> String {
    format!("{COOKIE_NAME}=; HttpOnly; Secure; SameSite=Lax; Path=/; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn someone() -> (SqliteDatabase, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let token = super::super::issue_setup_token(&database)
            .await
            .expect("token");
        let account = super::super::create_admin(&database, &token, "jorge", None)
            .await
            .expect("created");
        (database, account.id)
    }

    #[tokio::test]
    async fn the_code_works_once_and_only_after_approval() {
        let (database, account) = someone().await;
        let browser = open(&database, &account, "pk", "Firefox on Windows", None)
            .await
            .expect("opened");
        assert_eq!(
            redeem(&database, &browser, "000000").await.expect("read"),
            Redeemed::NotYet
        );

        let request = pending_for(&database, &account)
            .await
            .expect("read")
            .expect("pending");
        let code = approve(&database, &account, &request.id)
            .await
            .expect("approved")
            .expect("a code");

        assert!(matches!(
            redeem(&database, &browser, &code).await.expect("read"),
            Redeemed::Accepted(_)
        ));
        assert_eq!(
            redeem(&database, &browser, &code).await.expect("read"),
            Redeemed::Gone
        );
    }

    #[tokio::test]
    async fn five_wrong_codes_close_the_request() {
        let (database, account) = someone().await;
        let browser = open(&database, &account, "pk", "Chrome on Android", None)
            .await
            .expect("opened");
        let request = pending_for(&database, &account)
            .await
            .expect("read")
            .expect("pending");
        let code = approve(&database, &account, &request.id)
            .await
            .expect("approved")
            .expect("a code");
        let wrong = if code == "000000" { "111111" } else { "000000" };

        for _ in 0..4 {
            assert_eq!(
                redeem(&database, &browser, wrong).await.expect("read"),
                Redeemed::Wrong
            );
        }
        assert_eq!(
            redeem(&database, &browser, wrong).await.expect("read"),
            Redeemed::Gone
        );
        assert_eq!(
            redeem(&database, &browser, &code).await.expect("read"),
            Redeemed::Gone
        );
    }
}
