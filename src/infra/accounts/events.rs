//! What happened to an account. No email and no push, so an event is
//! kept where its owner looks: Your account › Activity, and a note on the
//! next page they open.

use wabot::sqlite::SqliteDatabase;

use super::AccountResult;
use crate::domain::values::account_event::AccountEvent;
use crate::infra::clock::now_ms;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    pub kind: AccountEvent,
    pub detail: String,
    pub address: Option<String>,
    pub at: i64,
    /// Not yet seen on the Activity tab.
    pub unread: bool,
}

/// `news` is false for what the owner just did themselves in this
/// console: telling them about it is noise, unless something they have
/// not read is already waiting.
pub async fn record(
    database: &SqliteDatabase,
    account_id: &str,
    kind: AccountEvent,
    detail: &str,
    address: Option<&str>,
    news: bool,
) -> AccountResult<()> {
    let row = (
        account_id.to_string(),
        kind.as_str(),
        detail.to_string(),
        address.map(str::to_string),
    );
    database
        .write(move |connection| {
            let now = now_ms();
            let unread: bool = connection.query_row(
                "SELECT EXISTS (SELECT 1 FROM account_event e JOIN account a ON a.\"id\" = e.\"account_id\" \
                 WHERE e.\"account_id\" = ?1 AND e.\"id\" > COALESCE(a.\"activity_seen_id\", 0))",
                [&row.0],
                |found| found.get(0),
            )?;
            connection.execute(
                "INSERT INTO account_event (\"account_id\", \"kind\", \"detail\", \"address\", \"at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (&row.0, row.1, &row.2, &row.3, now),
            )?;
            if !news && !unread {
                connection.execute(
                    "UPDATE account SET \"activity_seen_id\" = ?2 WHERE \"id\" = ?1",
                    (&row.0, connection.last_insert_rowid()),
                )?;
            }
            Ok(())
        })
        .await?;
    Ok(())
}

/// Record, and only log when it cannot: an event lost is worse than
/// nothing, but refusing what already happened is worse still.
pub async fn note(
    database: &SqliteDatabase,
    account_id: &str,
    kind: AccountEvent,
    detail: &str,
    address: Option<&str>,
    news: bool,
) {
    if let Err(error) = record(database, account_id, kind, detail, address, news).await {
        tracing::warn!(%error, account = account_id, "could not record an account event");
    }
}

pub async fn of_account(
    database: &SqliteDatabase,
    account_id: &str,
    limit: usize,
) -> AccountResult<Vec<Event>> {
    let account = account_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT e.\"kind\", e.\"detail\", e.\"address\", e.\"at\", \
                     e.\"id\" > COALESCE(a.\"activity_seen_id\", 0) \
                     FROM account_event e JOIN account a ON a.\"id\" = e.\"account_id\" \
                     WHERE e.\"account_id\" = ?1 ORDER BY e.\"at\" DESC, e.\"id\" DESC LIMIT ?2",
                )?
                .query_map((account, limit as i64), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()
        })
        .await?
        .into_iter()
        .filter_map(|(kind, detail, address, at, unread)| {
            Some(Event {
                kind: AccountEvent::parse(&kind)?,
                detail,
                address,
                at,
                unread,
            })
        })
        .collect())
}

pub async fn unseen(database: &SqliteDatabase, account_id: &str) -> AccountResult<usize> {
    let account = account_id.to_string();
    Ok(database
        .read(move |connection| {
            connection.query_row(
                "SELECT COUNT(*) FROM account_event e JOIN account a ON a.\"id\" = e.\"account_id\" \
                 WHERE e.\"account_id\" = ?1 AND e.\"id\" > COALESCE(a.\"activity_seen_id\", 0)",
                [account],
                |row| row.get::<_, i64>(0),
            )
        })
        .await? as usize)
}

pub async fn seen(database: &SqliteDatabase, account_id: &str) -> AccountResult<()> {
    let account = account_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE account SET \"activity_seen_id\" = \
                 (SELECT MAX(\"id\") FROM account_event WHERE \"account_id\" = ?1) \
                 WHERE \"id\" = ?1",
                [account],
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn what_the_owner_did_themselves_is_not_news_unless_news_is_waiting() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let token = super::super::issue_setup_token(&database)
            .await
            .expect("token");
        let account = super::super::create_admin(&database, &token, "jorge", None)
            .await
            .expect("created");

        record(
            &database,
            &account.id,
            AccountEvent::PasskeyAdded,
            "",
            None,
            false,
        )
        .await
        .expect("recorded");
        assert_eq!(unseen(&database, &account.id).await.expect("read"), 0);

        record(
            &database,
            &account.id,
            AccountEvent::LinkUsed,
            "root",
            None,
            true,
        )
        .await
        .expect("recorded");
        record(
            &database,
            &account.id,
            AccountEvent::PasskeyAdded,
            "",
            None,
            false,
        )
        .await
        .expect("recorded");
        assert_eq!(unseen(&database, &account.id).await.expect("read"), 2);

        seen(&database, &account.id).await.expect("seen");
        assert_eq!(unseen(&database, &account.id).await.expect("read"), 0);
        assert_eq!(
            of_account(&database, &account.id, 10)
                .await
                .expect("read")
                .len(),
            3
        );
    }
}
