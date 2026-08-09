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
//! ## Clamping
//!
//! `wg genkey` clamps before printing; `x25519-dalek` clamps at use.
//! Both arrive at the same public key from the same 32 bytes, and `wg`
//! clamps again on anything handed to it — so what is stored here is
//! interoperable with the tools without being pre-clamped.

use wabot::sqlite::{SqliteDatabase, SqliteResult};

use base64::Engine;
use x25519_dalek::{PublicKey, StaticSecret};

/// Where the private half lives in the `setting` table.
const PRIVATE_KEY: &str = "network.private_key";

/// This node's key pair, base64 as WireGuard writes it.
pub struct Keys {
    /// Nothing reads this yet. It is what phase 2 writes into a
    /// WireGuard interface, and it is returned here rather than hidden
    /// because a key pair is one thing — an accessor that hands out
    /// half of it would be a shape chosen by what happens to be
    /// implemented this week. The `allow` goes when the tunnel lands.
    #[allow(dead_code)]
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

    let secret = StaticSecret::random();
    let private = encode(&secret.to_bytes());
    crate::node::settings::write(database, PRIVATE_KEY, &private).await?;

    Ok(Keys {
        public: encode(PublicKey::from(&secret).as_bytes()),
        private,
    })
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
    let Some(bytes) = decode(&stored) else {
        tracing::warn!("the stored overlay key is not 32 bytes of base64; generating another");
        return Ok(None);
    };

    let secret = StaticSecret::from(bytes);
    Ok(Some(Keys {
        public: encode(PublicKey::from(&secret).as_bytes()),
        private: stored,
    }))
}

/// Standard base64 with padding — what `wg` prints and accepts.
fn encode(bytes: &[u8; 32]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(text: &str) -> Option<[u8; 32]> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .ok()?;
    bytes.try_into().ok()
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
            assert_eq!(decode(key).map(|bytes| bytes.len()), Some(32));
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
        assert_eq!(decode(&keys.private).map(|bytes| bytes.len()), Some(32));
    }
}
