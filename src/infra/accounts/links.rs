//! One-time ways in: the invitation's mechanism, issued by somebody above
//! the person or by one of their own trusted devices (`docs/access.md`,
//! "Links").

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{sha256_hex, AccountResult};
use crate::infra::clock::now_ms;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    SignIn,
    Device,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::SignIn => "sign-in",
            Self::Device => "device",
        }
    }

    fn parse(text: &str) -> Self {
        match text {
            "device" => Self::Device,
            _ => Self::SignIn,
        }
    }

    pub fn minutes(self) -> i64 {
        match self {
            Self::SignIn => 15,
            Self::Device => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub account_id: String,
    pub purpose: Purpose,
    pub issued_by: Option<String>,
    pub issued_by_name: Option<String>,
}

/// `issued_by` is `(id, username)`, and `None` is root on the terminal.
pub async fn issue(
    database: &SqliteDatabase,
    account_id: &str,
    purpose: Purpose,
    issued_by: Option<(&str, &str)>,
) -> AccountResult<String> {
    let token = wabot::prelude::password::generate(40);
    let row = (
        sha256_hex(&token),
        account_id.to_string(),
        purpose.as_str(),
        issued_by.map(|(id, _)| id.to_string()),
        issued_by.map(|(_, name)| name.to_string()),
        now_ms() + purpose.minutes() * 60_000,
    );
    database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM access_link WHERE \"expires_at\" < ?1",
                [now_ms()],
            )?;
            connection.execute(
                "INSERT INTO access_link (\"token_hash\", \"account_id\", \"purpose\", \
                 \"issued_by\", \"issued_by_name\", \"created_at\", \"expires_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (row.0, row.1, row.2, row.3, row.4, now_ms(), row.5),
            )?;
            Ok(())
        })
        .await?;
    Ok(token)
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Link> {
    Ok(Link {
        account_id: row.get(0)?,
        purpose: Purpose::parse(&row.get::<_, String>(1)?),
        issued_by: row.get(2)?,
        issued_by_name: row.get(3)?,
    })
}

/// What a link would do, without spending it: the page a link opens
/// shows whose it is, and a prefetcher must not use it up.
pub async fn look_up(database: &SqliteDatabase, token: &str) -> AccountResult<Option<Link>> {
    let hash = sha256_hex(token);
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"account_id\", \"purpose\", \"issued_by\", \"issued_by_name\" \
                     FROM access_link \
                     WHERE \"token_hash\" = ?1 AND \"used_at\" IS NULL AND \"expires_at\" >= ?2",
                    (hash, now_ms()),
                    decode,
                )
                .optional()
        })
        .await?)
}

pub async fn spend(database: &SqliteDatabase, token: &str) -> AccountResult<Option<Link>> {
    let hash = sha256_hex(token);
    Ok(database
        .write(move |connection| {
            let changed = connection.execute(
                "UPDATE access_link SET \"used_at\" = ?2 \
                 WHERE \"token_hash\" = ?1 AND \"used_at\" IS NULL AND \"expires_at\" >= ?2",
                (&hash, now_ms()),
            )?;
            if changed == 0 {
                return Ok(None);
            }
            connection
                .query_row(
                    "SELECT \"account_id\", \"purpose\", \"issued_by\", \"issued_by_name\" \
                     FROM access_link WHERE \"token_hash\" = ?1",
                    [&hash],
                    decode,
                )
                .optional()
        })
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_link_is_read_freely_and_spent_once() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let token = super::super::issue_setup_token(&database)
            .await
            .expect("token");
        let account = super::super::create_admin(&database, &token, "jorge", None)
            .await
            .expect("created");
        let link = issue(&database, &account.id, Purpose::SignIn, None)
            .await
            .expect("issued");

        assert!(look_up(&database, &link).await.expect("read").is_some());
        assert!(look_up(&database, &link).await.expect("read").is_some());
        assert_eq!(
            spend(&database, &link)
                .await
                .expect("spent")
                .map(|found| found.account_id),
            Some(account.id)
        );
        assert!(spend(&database, &link).await.expect("read").is_none());
        assert!(look_up(&database, &link).await.expect("read").is_none());
    }
}
