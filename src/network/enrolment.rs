//! Enrolment: a token, spent once, that turns a stranger into a node.
//!
//! The same mechanics as the setup token and the invitation, for the
//! same reasons — minted in clear once, stored hashed, time-limited —
//! with one difference that matters. An invitation is spent by a person
//! filling in a form, and a person who does not see the response tries
//! again by hand. This is spent by a machine over the network, where a
//! response that never arrives is indistinguishable from one that was
//! never sent. So spending is **idempotent for the node that spent it**:
//! the same node presenting the same token again is the same join, and
//! an errand sent twice must not fail the second time.

use serde::Serialize;
use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::SqliteDatabase;

use crate::accounts::sha256_hex;
use crate::network::NetworkResult;
use crate::platform::now_ms;

/// How long a token is worth anything.
///
/// A day, not the invitation's week: this one is a credential for a
/// machine, and the gap between minting it and pasting it into a
/// terminal is minutes. What the window really bounds is how long a
/// token left in somebody's scrollback stays a way onto the overlay.
const ENROLMENT_HOURS: i64 = 24;

/// A pending or spent enrolment, as the nodes page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Enrolment {
    pub id: String,
    pub name: String,
    pub overlay_ip: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub used_at: Option<i64>,
    /// The node that spent it, once one has.
    pub used_by: Option<String>,
}

impl Enrolment {
    pub fn spent(&self) -> bool {
        self.used_at.is_some()
    }

    pub fn expired(&self, now: i64) -> bool {
        self.expires_at <= now
    }

    /// Still worth carrying to a machine?
    pub fn live(&self, now: i64) -> bool {
        !self.spent() && !self.expired(now)
    }
}

/// Mint one, and return the secret in clear.
///
/// The only time it exists in clear, on its way into the token the
/// operator copies. What is stored is its hash.
pub async fn create(
    database: &SqliteDatabase,
    name: &str,
    overlay_ip: &str,
    created_by: &str,
) -> NetworkResult<(Enrolment, String)> {
    let secret = wabot::prelude::password::generate(40);
    let enrolment = Enrolment {
        id: format!("en-{}", wabot::prelude::password::generate(12)),
        name: name.trim().to_string(),
        overlay_ip: overlay_ip.to_string(),
        created_at: now_ms(),
        expires_at: now_ms() + ENROLMENT_HOURS * 3_600_000,
        used_at: None,
        used_by: None,
    };

    let row = enrolment.clone();
    let hash = sha256_hex(&secret);
    let creator = created_by.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO enrolment \
                   (\"id\", \"name\", \"token_hash\", \"overlay_ip\", \"created_by\", \
                    \"created_at\", \"expires_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                (
                    row.id,
                    row.name,
                    hash,
                    row.overlay_ip,
                    creator,
                    row.created_at,
                    row.expires_at,
                ),
            )?;
            Ok(())
        })
        .await?;

    Ok((enrolment, secret))
}

/// What this secret is worth, if anything.
///
/// Read-only, and it does not filter on `live`: a spent token presented
/// by the node that spent it is a retry, and the caller is the one that
/// can tell the difference. Expiry *is* filtered, because a token past
/// its window is nothing to anybody.
pub async fn look_up(database: &SqliteDatabase, secret: &str) -> NetworkResult<Option<Enrolment>> {
    let hash = sha256_hex(secret);
    let found: Option<Enrolment> = database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM enrolment WHERE \"token_hash\" = ?1"),
                    [hash],
                    decode,
                )
                .optional()
        })
        .await?;

    Ok(found.filter(|enrolment| !enrolment.expired(now_ms())))
}

/// Which node this secret belongs to, if any.
///
/// The join token becomes the standing credential of the node that
/// spent it: that is what the secret was always for — the doc calls it
/// "what an errand from the authority will carry" — and errands turned
/// out to be collected rather than delivered, so it is what the node
/// presents when it asks.
///
/// **Expiry is deliberately not checked.** The window bounds how long a
/// token can be used to *join*; a node that joined does not stop being
/// that node a day later. Filtering here would cut every node off from
/// its authority exactly 24 hours after it arrived.
pub async fn holder(database: &SqliteDatabase, secret: &str) -> NetworkResult<Option<String>> {
    let hash = sha256_hex(secret);
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"used_by\" FROM enrolment WHERE \"token_hash\" = ?1",
                    [hash],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
        })
        .await?
        .flatten())
}

/// Spend it for `node_id`, or say it is not this node's to spend.
///
/// The whole race, in one statement. A token that is unspent is taken;
/// a token already spent **by this same node** is taken again, which is
/// what makes a retried callback succeed. Anything else is refused, and
/// the refusal is the property the token exists for.
pub async fn spend(database: &SqliteDatabase, id: &str, node_id: &str) -> NetworkResult<bool> {
    let (id, node_id) = (id.to_string(), node_id.to_string());
    let taken = database
        .write(move |connection| {
            connection.execute(
                "UPDATE enrolment SET \"used_at\" = ?2, \"used_by\" = ?3 \
                 WHERE \"id\" = ?1 AND (\"used_at\" IS NULL OR \"used_by\" = ?3)",
                (id, now_ms(), node_id),
            )
        })
        .await?;
    Ok(taken > 0)
}

/// Every enrolment, newest first.
pub async fn all(database: &SqliteDatabase) -> NetworkResult<Vec<Enrolment>> {
    Ok(database
        .read(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {COLUMNS} FROM enrolment ORDER BY \"created_at\" DESC"
            ))?;
            let enrolments: wabot::sqlite::rusqlite::Result<Vec<Enrolment>> =
                statement.query_map([], decode)?.collect();
            enrolments
        })
        .await?)
}

/// Withdraw one, which also frees the address it was holding.
pub async fn withdraw(database: &SqliteDatabase, id: &str) -> NetworkResult<()> {
    let id = id.to_string();
    Ok(database
        .write(move |connection| {
            connection.execute("DELETE FROM enrolment WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?)
}

const COLUMNS: &str = "\"id\", \"name\", \"overlay_ip\", \"created_at\", \"expires_at\", \
                       \"used_at\", \"used_by\"";

fn decode(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Enrolment> {
    Ok(Enrolment {
        id: row.get(0)?,
        name: row.get(1)?,
        overlay_ip: row.get(2)?,
        created_at: row.get(3)?,
        expires_at: row.get(4)?,
        used_at: row.get(5)?,
        used_by: row.get(6)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn node() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let admin = crate::network::tests::admin(&database).await;
        (database, admin)
    }

    async fn mint(database: &SqliteDatabase, admin: &str) -> (Enrolment, String) {
        create(database, "alpine", "10.42.0.2", admin)
            .await
            .expect("minted")
    }

    #[tokio::test]
    async fn a_token_names_the_enrolment_it_belongs_to() {
        let (database, admin) = node().await;
        let (enrolment, secret) = mint(&database, &admin).await;

        let found = look_up(&database, &secret)
            .await
            .expect("look up")
            .expect("there");
        assert_eq!(found, enrolment);
        assert!(found.live(now_ms()));
        assert_eq!(found.overlay_ip, "10.42.0.2");
    }

    #[tokio::test]
    async fn something_that_is_not_a_token_is_nothing() {
        let (database, admin) = node().await;
        mint(&database, &admin).await;

        assert_eq!(look_up(&database, "made-up").await.expect("look up"), None);
    }

    /// The property the token exists for: one node joins with it, and
    /// the next one is refused.
    #[tokio::test]
    async fn a_token_admits_one_node() {
        let (database, admin) = node().await;
        let (enrolment, _) = mint(&database, &admin).await;

        assert!(spend(&database, &enrolment.id, "nd-first")
            .await
            .expect("spend"));
        assert!(
            !spend(&database, &enrolment.id, "nd-second")
                .await
                .expect("spend"),
            "a second node took a token that was already spent"
        );
    }

    /// A callback whose response was lost is re-sent, and the same node
    /// arriving twice is one join. This is the difference between a
    /// token spent by a person and one spent by a machine.
    #[tokio::test]
    async fn the_node_that_spent_it_may_spend_it_again() {
        let (database, admin) = node().await;
        let (enrolment, _) = mint(&database, &admin).await;

        for _ in 0..3 {
            assert!(spend(&database, &enrolment.id, "nd-first")
                .await
                .expect("spend"));
        }
    }

    #[tokio::test]
    async fn an_expired_token_is_worth_nothing() {
        let (database, admin) = node().await;
        let (enrolment, secret) = mint(&database, &admin).await;

        let id = enrolment.id.clone();
        database
            .write(move |connection| {
                connection.execute(
                    "UPDATE enrolment SET \"expires_at\" = 1 WHERE \"id\" = ?1",
                    [id],
                )
            })
            .await
            .expect("expire");

        assert_eq!(look_up(&database, &secret).await.expect("look up"), None);
    }

    /// Withdrawing frees the address, so a token minted by mistake does
    /// not cost the overlay an address for ever.
    #[tokio::test]
    async fn withdrawing_gives_the_address_back() {
        let (database, admin) = node().await;
        let (enrolment, secret) = mint(&database, &admin).await;

        withdraw(&database, &enrolment.id).await.expect("withdraw");

        assert_eq!(look_up(&database, &secret).await.expect("look up"), None);
        assert_eq!(
            crate::network::overlay::allocate(&database)
                .await
                .expect("allocate"),
            "10.42.0.1",
            "the address it was holding is free again"
        );
    }

    /// The token becomes the standing credential of whoever spent it,
    /// and the expiry is not rechecked — the window bounds joining, not
    /// being a node afterwards. Filtering here would cut every node off
    /// from its authority a day after it arrived.
    #[tokio::test]
    async fn a_spent_token_names_its_node_for_ever() {
        let (database, admin) = node().await;
        let (enrolment, secret) = mint(&database, &admin).await;

        assert_eq!(
            holder(&database, &secret).await.expect("holder"),
            None,
            "nobody has spent it yet"
        );
        spend(&database, &enrolment.id, "nd-first")
            .await
            .expect("spend");
        assert_eq!(
            holder(&database, &secret).await.expect("holder").as_deref(),
            Some("nd-first")
        );

        let id = enrolment.id.clone();
        database
            .write(move |connection| {
                connection.execute(
                    "UPDATE enrolment SET \"expires_at\" = 1 WHERE \"id\" = ?1",
                    [id],
                )
            })
            .await
            .expect("expire");
        assert_eq!(
            holder(&database, &secret).await.expect("holder").as_deref(),
            Some("nd-first"),
            "the node was cut off a day after it joined"
        );
        assert_eq!(holder(&database, "made-up").await.expect("holder"), None);
    }

    /// A database somebody reads must not be a database somebody joins
    /// with.
    #[tokio::test]
    async fn the_secret_is_not_stored() {
        let (database, admin) = node().await;
        let (_, secret) = mint(&database, &admin).await;

        let stored: String = database
            .read(|connection| {
                connection.query_row("SELECT \"token_hash\" FROM enrolment", [], |row| row.get(0))
            })
            .await
            .expect("query");
        assert_ne!(stored, secret);
        assert_eq!(stored, sha256_hex(&secret));
    }

    #[tokio::test]
    async fn the_list_says_which_ones_are_spent() {
        let (database, admin) = node().await;
        let (first, _) = mint(&database, &admin).await;
        create(&database, "another", "10.42.0.3", &admin)
            .await
            .expect("minted");

        spend(&database, &first.id, "nd-first")
            .await
            .expect("spend");

        let all = all(&database).await.expect("list");
        assert_eq!(all.len(), 2);
        assert_eq!(all.iter().filter(|e| e.spent()).count(), 1);
        assert_eq!(all.iter().filter(|e| e.live(now_ms())).count(), 1);
        assert_eq!(
            all.iter()
                .find(|e| e.spent())
                .and_then(|e| e.used_by.as_deref()),
            Some("nd-first"),
            "and which node spent it"
        );
    }
}
