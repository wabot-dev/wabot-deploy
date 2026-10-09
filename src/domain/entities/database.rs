//! A managed database: an engine, a version, and the credentials the
//! node minted for it.
//!
//! **A name belongs to the database, not to the machine holding a copy.**
//! `owner_domain` is why: the long name used to be built from the local
//! node's domain, so the same database answered to a different qualified
//! name on each machine and its certificate matched neither.

use serde::Serialize;

use crate::domain::policies::postgres;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Postgres,
    /// A row written by a newer node. Held and refused, never parsed
    /// into a panic.
    Unknown,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Postgres => "postgres",
            Engine::Unknown => "unknown",
        }
    }

    /// What the console calls it. Not translated — it is a product
    /// name.
    pub fn label(self) -> &'static str {
        match self {
            Engine::Postgres => "PostgreSQL",
            Engine::Unknown => "unknown",
        }
    }

    pub fn parse(text: &str) -> Self {
        match text {
            "postgres" => Engine::Postgres,
            _ => Engine::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Database {
    pub service_id: String,
    pub engine: Engine,
    /// The major version, as the image tags spell it.
    pub version: String,
    pub admin_user: String,
    pub admin_password: String,
    pub database_name: String,
    pub replication_user: String,
    pub replication_password: String,
    /// Which slot accepts writes.
    pub primary_slot: u32,
    /// Where the primary answers, when this node was **told** rather
    /// than being able to work it out.
    ///
    /// `None` on the node that owns the database: it derives the
    /// address from its own rows, and a stored copy would be a second
    /// answer able to go stale. `Some` on a node holding a standby,
    /// which has no row for the primary and never will.
    pub primary_endpoint: Option<String>,
    /// Whose domain the qualified name is built from.
    ///
    /// `None` is this node's own. `Some` on a copy held for somebody
    /// else — see migration `0032`: a name that changed with the
    /// machine holding it was no name at all.
    pub owner_domain: Option<String>,
    /// The backup this was restored from, and the moment it was asked to
    /// stop at. Provenance rather than state: Postgres clears
    /// `recovery.signal` itself when recovery finishes, and these stay so
    /// that somebody reading the row in three weeks can tell where it
    /// came from. See `migrations/0041_restored_from.sql`.
    pub restored_from: Option<String>,
    pub recovery_target: Option<String>,
}

impl Database {
    /// Whether this copy is the one that accepts writes.
    /// What this copy is, which decides what a deployment does with an
    /// empty volume.
    ///
    /// **Restoring outranks primary.** A database being restored has a
    /// primary slot like any other, and if that were asked first the
    /// deploy path would run `initdb` into the volume a base backup was
    /// meant to be unpacked into — producing an empty database that
    /// starts, answers, and looks exactly like a restore that worked.
    ///
    /// It stops being `Restoring` when the volume holds a cluster:
    /// unpacking happened, recovery is Postgres's business from there,
    /// and `recovery.signal` is deleted by Postgres itself when it
    /// finishes. So this is about the *unpack*, not about the replay.
    pub fn role_of(&self, slot: u32) -> postgres::Role {
        if self.restored_from.is_some() && slot == self.primary_slot {
            return postgres::Role::Restoring;
        }
        match slot == self.primary_slot {
            true => postgres::Role::Primary,
            false => postgres::Role::Standby,
        }
    }
}

/// A slug as a Postgres identifier.
///
/// Hyphens become underscores and a leading digit gets a prefix,
/// because an identifier that needs quoting is one every client has to
/// remember to quote — and the one place it will be forgotten is a
/// connection string somebody types by hand.
pub(crate) fn identifier(slug: &str) -> String {
    let mut identifier: String = slug
        .chars()
        .map(|c| match c.is_ascii_alphanumeric() {
            true => c.to_ascii_lowercase(),
            false => '_',
        })
        .collect();

    if identifier.chars().next().is_none_or(|c| c.is_ascii_digit()) {
        identifier.insert_str(0, "db_");
    }
    // Postgres truncates at 63 bytes and says nothing, so the name in
    // the connection string would stop matching the one it made.
    identifier.truncate(63);
    identifier
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An identifier that needs quoting is one every client has to
    /// remember to quote, and the place it gets forgotten is a
    /// connection string somebody types.
    #[test]
    fn an_identifier_never_needs_quoting() {
        assert_eq!(identifier("orders"), "orders");
        assert_eq!(identifier("orders-api"), "orders_api");
        assert_eq!(identifier("2fa"), "db_2fa");
        assert_eq!(identifier(""), "db_");
        assert_eq!(identifier(&"a".repeat(100)).len(), 63);

        for slug in ["orders", "orders-api", "2fa", "x-1-y"] {
            let identifier = identifier(slug);
            assert!(
                identifier
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{identifier}"
            );
            assert!(!identifier.starts_with(|c: char| c.is_ascii_digit()));
        }
    }
}
