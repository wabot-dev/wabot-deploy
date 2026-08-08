//! How each name's certificate is kept.
//!
//! ## Why this is not a column on the certificate
//!
//! A certificate row records what was obtained. A policy records what
//! to do next, and the two have different lifetimes: a name is
//! configured before anything exists for it, and an operator's choice
//! has to survive the certificate being replaced. Folding them
//! together would mean a row with no certificate in it, which is a
//! certificate table that lies about what it holds.
//!
//! ## Absent is a value
//!
//! No row means [`RenewWith::default_for`], so a node nobody has
//! configured needs no rows at all and behaves exactly as it did
//! before this existed. Writing a row is how somebody departs from the
//! default, which is the only time the choice is interesting.
//!
//! ## The node cannot renew what it did not issue
//!
//! [`RenewWith::File`] is the honest form of "renew the certificate I
//! gave you". The node has no relationship with whoever signed it, so
//! it cannot ask for another — but something else can keep the files
//! fresh, and the node can reinstall what it finds. Everything else is
//! replacement rather than renewal, which is what [`RenewWith::Acme`]
//! and [`RenewWith::SelfSigned`] are.

use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::{SqliteDatabase, SqliteResult};

use crate::config::Config;
use crate::platform::now_ms;

/// What to do about a name's certificate when it needs one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenewWith {
    /// Ask a public authority. The default wherever ACME can work.
    Acme,
    /// Sign it here. For a name no public authority can validate — a
    /// private network, a node reachable only from inside.
    SelfSigned,
    /// Read it from disk, and reinstall it whenever what is there
    /// stops matching what is served.
    File { cert_path: String, key_path: String },
}

impl RenewWith {
    /// What a name gets when nobody has chosen.
    ///
    /// Reads the config rather than hard-coding ACME: `acme.disabled`
    /// is an operator saying up front that this node signs for itself,
    /// and a default that ignored it would start ordering certificates
    /// they had switched off.
    pub fn default_for(config: &Config) -> Self {
        if config.acme.disabled {
            Self::SelfSigned
        } else {
            Self::Acme
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Acme => "acme",
            Self::SelfSigned => "self_signed",
            Self::File { .. } => "file",
        }
    }

    /// Whether the node can produce a replacement on its own.
    ///
    /// `File` cannot: if the files stop being refreshed, nothing here
    /// can make a new certificate appear at that path. That is what
    /// makes the expiry floor necessary rather than merely tidy.
    pub fn is_self_serve(&self) -> bool {
        !matches!(self, Self::File { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub name: String,
    pub renew_with: RenewWith,
    /// Why the last attempt for this name produced nothing, if it did
    /// not. Per name rather than per node: one `acme_error` shown
    /// against every hostname is a reason attached to names it was
    /// never about.
    pub last_error: Option<String>,
}

/// Read a row, with the caller's default standing in for NULL.
///
/// NULL is not "no policy" — it is "the default, whatever that resolves
/// to now". A row can exist purely to carry a failure, and resolving it
/// here keeps that row from asserting a choice nobody made.
fn read(row: &Row<'_>, default: &RenewWith) -> wabot::sqlite::rusqlite::Result<Policy> {
    let name: String = row.get(0)?;
    let kind: Option<String> = row.get(1)?;
    let cert_path: Option<String> = row.get(2)?;
    let key_path: Option<String> = row.get(3)?;
    let last_error: Option<String> = row.get(4)?;

    let Some(kind) = kind else {
        return Ok(Policy {
            name,
            renew_with: default.clone(),
            last_error,
        });
    };

    let renew_with = match kind.as_str() {
        "self_signed" => RenewWith::SelfSigned,
        // Both paths or neither. A half-configured file source would
        // otherwise read as "file" and then find nothing to read, on
        // every pass, for as long as the row survives.
        "file" => match (cert_path, key_path) {
            (Some(cert_path), Some(key_path)) => RenewWith::File {
                cert_path,
                key_path,
            },
            _ => default.clone(),
        },
        // Including anything unrecognised. A row written by a newer
        // version must not stop this one from getting a certificate.
        _ => default.clone(),
    };
    Ok(Policy {
        name,
        renew_with,
        last_error,
    })
}

const COLUMNS: &str = "\"name\", \"renew_with\", \"cert_path\", \"key_path\", \"last_error\"";

/// What is configured for `name`, or the default.
pub async fn for_name(database: &SqliteDatabase, config: &Config, name: &str) -> Policy {
    let default = RenewWith::default_for(config);
    let fallback = || Policy {
        name: name.to_string(),
        renew_with: default.clone(),
        last_error: None,
    };

    match stored(database, name, &default).await {
        Ok(Some(policy)) => policy,
        Ok(None) => fallback(),
        Err(error) => {
            // A policy nobody can read is not a reason to stop serving.
            // The default is what this name had before anybody chose.
            tracing::warn!(%name, %error, "could not read the certificate policy");
            fallback()
        }
    }
}

async fn stored(
    database: &SqliteDatabase,
    name: &str,
    default: &RenewWith,
) -> SqliteResult<Option<Policy>> {
    let (name, default) = (name.to_string(), default.clone());
    database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM certificate_policy WHERE \"name\" = ?1"),
                    [name],
                    |row| read(row, &default),
                )
                .optional()
        })
        .await
}

/// Record why `name` has no certificate, where the page about that name
/// will look.
pub async fn record_failure(
    database: &SqliteDatabase,
    name: &str,
    reason: &str,
) -> SqliteResult<()> {
    upsert_failure(database, name, Some(reason.to_string())).await
}

/// Forget it, which is what a certificate arriving means.
pub async fn clear_failure(database: &SqliteDatabase, name: &str) -> SqliteResult<()> {
    upsert_failure(database, name, None).await
}

async fn upsert_failure(
    database: &SqliteDatabase,
    name: &str,
    reason: Option<String>,
) -> SqliteResult<()> {
    let name = name.to_string();
    database
        .write(move |connection| {
            // `renew_with` stays NULL on insert: this row exists to
            // carry a failure, and writing a choice here would invent
            // one the operator never made.
            connection.execute(
                "INSERT INTO certificate_policy \
                   (\"name\", \"renew_with\", \"last_error\", \"updated_at\") \
                 VALUES (?1, NULL, ?2, ?3) \
                 ON CONFLICT (\"name\") DO UPDATE SET \
                   \"last_error\" = excluded.\"last_error\", \
                   \"updated_at\" = excluded.\"updated_at\"",
                (name, reason, now_ms()),
            )?;
            Ok(())
        })
        .await
}

pub async fn set(
    database: &SqliteDatabase,
    name: &str,
    renew_with: &RenewWith,
) -> SqliteResult<()> {
    let (name, kind) = (name.to_string(), renew_with.as_str().to_string());
    let (cert_path, key_path) = match renew_with {
        RenewWith::File {
            cert_path,
            key_path,
        } => (Some(cert_path.clone()), Some(key_path.clone())),
        _ => (None, None),
    };

    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO certificate_policy \
                   (\"name\", \"renew_with\", \"cert_path\", \"key_path\", \"updated_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5) \
                 ON CONFLICT (\"name\") DO UPDATE SET \
                   \"renew_with\" = excluded.\"renew_with\", \
                   \"cert_path\"  = excluded.\"cert_path\", \
                   \"key_path\"   = excluded.\"key_path\", \
                   \"updated_at\" = excluded.\"updated_at\"",
                (name, kind, cert_path, key_path, now_ms()),
            )?;
            Ok(())
        })
        .await
}

/// Go back to the default by forgetting the choice, rather than by
/// writing the default down. A stored default would go stale the day
/// `acme.disabled` changed.
pub async fn clear(database: &SqliteDatabase, name: &str) -> SqliteResult<()> {
    let name = name.to_string();
    database
        .write(move |connection| {
            // The choice goes, the failure stays: they are two facts,
            // and going back to the default does not un-refuse the
            // last attempt.
            connection.execute(
                "UPDATE certificate_policy SET \"renew_with\" = NULL, \"cert_path\" = NULL, \
                   \"key_path\" = NULL, \"updated_at\" = ?2 WHERE \"name\" = ?1",
                (name.clone(), now_ms()),
            )?;
            // And a row left holding nothing at all is a row nobody
            // needs.
            connection.execute(
                "DELETE FROM certificate_policy \
                 WHERE \"name\" = ?1 AND \"renew_with\" IS NULL AND \"last_error\" IS NULL",
                [name],
            )?;
            Ok(())
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
    }

    /// The whole point of "absent is a value": a node nobody has
    /// configured has no rows and behaves as it always did.
    #[tokio::test]
    async fn a_name_nobody_configured_gets_the_default() {
        let database = database().await;
        let mut config = Config::default();

        assert_eq!(
            for_name(&database, &config, "node.example.com")
                .await
                .renew_with,
            RenewWith::Acme
        );

        // Switching ACME off is somebody saying this node signs for
        // itself. A default that ignored it would order certificates
        // they had turned off.
        config.acme.disabled = true;
        assert_eq!(
            for_name(&database, &config, "node.example.com")
                .await
                .renew_with,
            RenewWith::SelfSigned
        );
    }

    #[tokio::test]
    async fn a_choice_is_stored_and_read_back() {
        let database = database().await;
        let config = Config::default();
        let file = RenewWith::File {
            cert_path: "/etc/ssl/node.crt".into(),
            key_path: "/etc/ssl/node.key".into(),
        };

        set(&database, "node.example.com", &file)
            .await
            .expect("set");
        assert_eq!(
            for_name(&database, &config, "node.example.com")
                .await
                .renew_with,
            file
        );

        // Clearing forgets rather than writes the default down: a
        // stored default goes stale the day `acme.disabled` changes.
        clear(&database, "node.example.com").await.expect("clear");
        assert_eq!(
            for_name(&database, &config, "node.example.com")
                .await
                .renew_with,
            RenewWith::Acme
        );
    }

    /// A row that says `file` and carries no path would find nothing to
    /// read, on every pass, forever. Falling back to the default gets
    /// the name a certificate instead.
    #[tokio::test]
    async fn a_half_configured_file_source_is_not_a_file_source() {
        let database = database().await;
        let config = Config::default();
        database
            .write(|connection| {
                connection.execute(
                    "INSERT INTO certificate_policy \
                       (\"name\", \"renew_with\", \"cert_path\", \"key_path\", \"updated_at\") \
                     VALUES ('node.example.com', 'file', '/etc/ssl/node.crt', NULL, 0)",
                    [],
                )?;
                Ok(())
            })
            .await
            .expect("insert");

        assert_eq!(
            for_name(&database, &config, "node.example.com")
                .await
                .renew_with,
            RenewWith::Acme
        );
    }

    /// The reason belongs to the name. One `acme_error` for the node
    /// was a reason shown against hostnames it was never about.
    #[tokio::test]
    async fn a_failure_is_recorded_against_one_name() {
        let database = database().await;
        let config = Config::default();

        record_failure(&database, "api.example.com", "dns did not answer")
            .await
            .expect("record");

        assert_eq!(
            for_name(&database, &config, "api.example.com")
                .await
                .last_error
                .as_deref(),
            Some("dns did not answer")
        );
        assert!(
            for_name(&database, &config, "other.example.com")
                .await
                .last_error
                .is_none(),
            "and to no other"
        );

        clear_failure(&database, "api.example.com")
            .await
            .expect("clear");
        assert!(for_name(&database, &config, "api.example.com")
            .await
            .last_error
            .is_none());
    }

    /// A row that exists only to carry a failure must not also assert a
    /// choice: `renew_with` stays NULL, so the answer still follows the
    /// config the day it changes.
    #[tokio::test]
    async fn a_recorded_failure_does_not_invent_a_choice() {
        let database = database().await;
        let mut config = Config::default();

        record_failure(&database, "api.example.com", "refused")
            .await
            .expect("record");
        assert_eq!(
            for_name(&database, &config, "api.example.com")
                .await
                .renew_with,
            RenewWith::Acme
        );

        config.acme.disabled = true;
        assert_eq!(
            for_name(&database, &config, "api.example.com")
                .await
                .renew_with,
            RenewWith::SelfSigned,
            "the row followed the config rather than freezing it"
        );
    }

    /// Going back to the default does not un-refuse the last attempt:
    /// the choice and the failure are two facts.
    #[tokio::test]
    async fn clearing_a_choice_keeps_the_reason() {
        let database = database().await;
        let config = Config::default();

        set(
            &database,
            "api.example.com",
            &RenewWith::File {
                cert_path: "/a".into(),
                key_path: "/b".into(),
            },
        )
        .await
        .expect("set");
        record_failure(&database, "api.example.com", "no such file")
            .await
            .expect("record");

        clear(&database, "api.example.com").await.expect("clear");
        let policy = for_name(&database, &config, "api.example.com").await;
        assert_eq!(policy.renew_with, RenewWith::Acme, "the choice went");
        assert_eq!(
            policy.last_error.as_deref(),
            Some("no such file"),
            "the reason stayed"
        );
    }

    /// Only a file source cannot produce its own replacement, and that
    /// is what the expiry floor exists for.
    #[test]
    fn only_a_file_source_needs_somebody_else() {
        assert!(RenewWith::Acme.is_self_serve());
        assert!(RenewWith::SelfSigned.is_self_serve());
        assert!(!RenewWith::File {
            cert_path: "a".into(),
            key_path: "b".into()
        }
        .is_self_serve());
    }
}
