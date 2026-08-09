//! The node's overlay identity: one Curve25519 key pair, kept.
//!
//! ## Why a key exists before there is an overlay
//!
//! Nothing dials anything yet — the tunnel is the next phase. The key
//! is here because it travels **in the join token**, and the token
//! format is the one thing in this design that would be expensive to
//! change later: a field added afterwards means every node that already
//! joined has to join again. A key generated now costs 32 bytes and
//! settles that.
//!
//! It also commits to nothing. A WireGuard key is a Curve25519 key
//! whichever implementation ends up carrying the packets — the kernel
//! module and `boringtun` read the same base64 — so this is not the
//! decision the spike in phase 2 is for.
//!
//! ## Private in the database, public in the node row
//!
//! The same split the certificates already use: `edge::certs` stores
//! its private keys here too, on a data directory created `0700`. The
//! public half goes in this node's `node` row, because that table is
//! what is *knowable about a node* — every peer's public key lives
//! there, and no peer's private one ever could.
//!
//! ## The same type the interface is configured with
//!
//! `Key` comes from the crate that talks netlink to the kernel, so the
//! key this generates is the key that is handed to WireGuard without a
//! conversion in between. It also means one copy of curve25519 in the
//! binary instead of two — which is what a separate x25519 dependency
//! cost, for the same arithmetic.
//!
//! The stored format is unchanged: 32 bytes, standard base64, exactly
//! what `wg genkey` prints. Keys generated before this are still read.

use defguard_wireguard_rs::key::Key;
use wabot::sqlite::{SqliteDatabase, SqliteResult};

/// Where the private half lives in the `setting` table.
const PRIVATE_KEY: &str = "network.private_key";

/// This node's key pair, base64 as WireGuard writes it.
pub struct Keys {
    /// What the WireGuard interface is configured with.
    pub private: String,
    pub public: String,
}

/// Read the key pair, generating one the first time.
///
/// Convergent, like every other step here: it asks whether this node
/// has a key, not whether anything has run before. Generating a second
/// one would invalidate every token already handed out.
pub async fn ensure(database: &SqliteDatabase) -> SqliteResult<Keys> {
    if let Some(keys) = read(database).await? {
        return Ok(keys);
    }

    let secret = Key::generate();
    let private = secret.to_string();
    crate::node::settings::write(database, PRIVATE_KEY, &private).await?;

    Ok(Keys {
        public: secret.public_key().to_string(),
        private,
    })
}

/// The private half, for the one caller that configures an interface
/// with it.
///
/// Read-only and `Option`, like [`public_key`]: a node with no key is
/// a node not on an overlay, and minting one here would be the tunnel
/// deciding something enrolment owns.
pub async fn private_key(database: &SqliteDatabase) -> Option<String> {
    match read(database).await {
        Ok(keys) => keys.map(|keys| keys.private),
        Err(error) => {
            tracing::warn!(%error, "could not read this node's overlay key");
            None
        }
    }
}

/// What this node's public key is, if it has one yet.
///
/// Read-only, so a page can report the key without minting one as a
/// side effect of being looked at.
pub async fn public_key(database: &SqliteDatabase) -> Option<String> {
    match read(database).await {
        Ok(keys) => keys.map(|keys| keys.public),
        Err(error) => {
            tracing::warn!(%error, "could not read this node's overlay key");
            None
        }
    }
}

/// The stored pair, or `None` when nothing has been generated.
///
/// A stored value that will not decode is treated as absent rather than
/// as an error: the only way to get one is somebody editing the row by
/// hand, and refusing for ever afterwards would be worse than minting
/// a key that works.
async fn read(database: &SqliteDatabase) -> SqliteResult<Option<Keys>> {
    let Some(stored) = crate::node::settings::read(database, PRIVATE_KEY).await? else {
        return Ok(None);
    };
    let Ok(secret) = Key::try_from(stored.trim()) else {
        tracing::warn!("the stored overlay key is not 32 bytes of base64; generating another");
        return Ok(None);
    };

    Ok(Some(Keys {
        public: secret.public_key().to_string(),
        private: stored,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generating a second key would silently invalidate every token
    /// already handed out — the public half is *in* them.
    #[tokio::test]
    async fn a_node_has_one_key_for_ever() {
        let database = crate::db::open_in_memory().await.expect("open");

        let first = ensure(&database).await.expect("generated");
        let second = ensure(&database).await.expect("read back");

        assert_eq!(first.private, second.private);
        assert_eq!(first.public, second.public);
        assert_eq!(public_key(&database).await.as_deref(), Some(&*first.public));
    }

    /// The shape `wg` reads: 32 bytes, base64, and the public half
    /// derived from the private one rather than stored beside it.
    #[tokio::test]
    async fn the_public_key_is_derived_and_wireguard_shaped() {
        let database = crate::db::open_in_memory().await.expect("open");
        let keys = ensure(&database).await.expect("generated");

        for key in [&keys.private, &keys.public] {
            assert_eq!(key.len(), 44, "{key} is not 32 base64 bytes");
            assert!(Key::try_from(key.as_str()).is_ok(), "{key}");
        }
        assert_ne!(keys.private, keys.public);
    }

    /// A node that has never been asked has no key, and asking about it
    /// must not create one — otherwise a page that merely reports the
    /// key is a page that mints it.
    #[tokio::test]
    async fn reading_does_not_generate() {
        let database = crate::db::open_in_memory().await.expect("open");
        assert_eq!(public_key(&database).await, None);
        assert_eq!(public_key(&database).await, None);
    }

    /// Nothing else in this crate can produce a bad value, so the only
    /// way to get one is by hand — and refusing for ever afterwards
    /// would be worse than replacing it.
    #[tokio::test]
    async fn a_key_that_is_not_a_key_is_replaced() {
        let database = crate::db::open_in_memory().await.expect("open");
        crate::node::settings::write(&database, PRIVATE_KEY, "not a key")
            .await
            .expect("write");

        let keys = ensure(&database).await.expect("generated");
        assert!(Key::try_from(keys.private.as_str()).is_ok());
    }
}
