//! A ceremony's challenge, between the page that rendered it and the POST
//! that answers it. Spent on use, so an answer cannot be replayed.

use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::AccountResult;
use crate::infra::clock::now_ms;

const LIFETIME_MS: i64 = 5 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Register,
    SignIn,
    Verify,
}

impl Purpose {
    fn as_str(self) -> &'static str {
        match self {
            Self::Register => "register",
            Self::SignIn => "sign-in",
            Self::Verify => "verify",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Challenge {
    pub value: String,
    pub account_id: Option<String>,
    pub user_handle: Option<String>,
}

pub async fn issue(
    database: &SqliteDatabase,
    purpose: Purpose,
    account_id: Option<&str>,
    user_handle: Option<&str>,
) -> AccountResult<String> {
    let value = wabot_addon_auth_passkey::challenge();
    let row = (
        value.clone(),
        purpose.as_str(),
        account_id.map(str::to_string),
        user_handle.map(str::to_string),
    );
    database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM passkey_challenge WHERE \"expires_at\" < ?1",
                [now_ms()],
            )?;
            connection.execute(
                "INSERT INTO passkey_challenge \
                 (\"challenge\", \"purpose\", \"account_id\", \"user_handle\", \"expires_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (row.0, row.1, row.2, row.3, now_ms() + LIFETIME_MS),
            )?;
            Ok(())
        })
        .await?;
    Ok(value)
}

/// The challenge, if it was issued for this and is still live. Gone
/// afterwards either way.
pub async fn spend(
    database: &SqliteDatabase,
    value: &str,
    purpose: Purpose,
) -> AccountResult<Option<Challenge>> {
    let (value, purpose) = (value.to_string(), purpose.as_str());
    Ok(database
        .write(move |connection| {
            let found = connection
                .query_row(
                    "SELECT \"account_id\", \"user_handle\" FROM passkey_challenge \
                     WHERE \"challenge\" = ?1 AND \"purpose\" = ?2 AND \"expires_at\" >= ?3",
                    (&value, purpose, now_ms()),
                    |row| {
                        Ok(Challenge {
                            value: value.clone(),
                            account_id: row.get(0)?,
                            user_handle: row.get(1)?,
                        })
                    },
                )
                .optional()?;
            connection.execute(
                "DELETE FROM passkey_challenge WHERE \"challenge\" = ?1",
                [&value],
            )?;
            Ok(found)
        })
        .await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_challenge_is_spent_once_and_only_for_what_it_was_issued_for() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let value = issue(&database, Purpose::SignIn, None, None)
            .await
            .expect("issued");
        assert!(spend(&database, &value, Purpose::Verify)
            .await
            .expect("read")
            .is_none());

        let other = issue(&database, Purpose::SignIn, None, None)
            .await
            .expect("issued");
        assert!(spend(&database, &other, Purpose::SignIn)
            .await
            .expect("read")
            .is_some());
        assert!(spend(&database, &other, Purpose::SignIn)
            .await
            .expect("read")
            .is_none());
    }
}
