//! Joining, as one piece of work with two front doors.
//!
//! A token can be pasted into a terminal or into this node's own
//! console, and the two must do exactly the same thing — the order of
//! the writes here is a safety property (see below), not a detail of
//! how somebody happened to type it. So the sequence lives here and
//! `commands::join` and the console are both thin: one prints the
//! result, the other renders it.
//!
//! ## Order matters
//!
//! The call goes out **before** the grant is written. Both orders leave
//! something to tidy up if the exchange half-fails, and this is the one
//! whose leftovers are harmless: an authority that knows about a node
//! which is not yet obeying it can be re-joined, whereas a node that
//! obeys an authority it never reached is one that granted power over
//! itself on the strength of a string somebody pasted.
//!
//! Both halves are convergent, so the fix for either is to do it again
//! with the same token.

use wabot::sqlite::SqliteDatabase;

use super::api::Arriving;
use super::token::{JoinToken, TokenError};
use super::{call, keys, Kind, Node};
use crate::config::Config;
use crate::platform::now_ms;

/// What a completed join left behind, for whoever has to report it.
#[derive(Debug)]
pub struct Joined {
    /// The node that may now send this one errands.
    pub authority: Node,
    /// This node, as it now stands — with its address on the overlay.
    pub me: Node,
    /// This node's overlay public key, which the authority recorded.
    pub public_key: String,
}

#[derive(Debug, thiserror::Error)]
pub enum JoinError {
    #[error(transparent)]
    Token(#[from] TokenError),
    #[error(transparent)]
    Call(#[from] call::CallError),
    /// The endpoint in the token does not lead to the node the token
    /// names. Granting authority to whatever answered is exactly what
    /// this refusal exists to prevent.
    #[error(
        "{endpoint} answered as {answered}, but the token names {expected} — nothing was granted"
    )]
    NotWhoItClaimed {
        endpoint: String,
        answered: String,
        expected: String,
    },
    #[error(transparent)]
    Storage(#[from] super::NetworkError),
}

/// Take instructions from the node that minted `token`.
pub async fn join(
    database: &SqliteDatabase,
    config: &Config,
    token: &str,
    accepting: Option<&[super::capability::Capability]>,
) -> Result<Joined, JoinError> {
    let token = JoinToken::decode(token)?;

    // What this node agrees to, out of what the token asked for.
    // `None` is "whatever it asked" — the terminal path, where there is
    // nobody to show a screen to and running the command *is* the
    // consent. The console passes what somebody ticked, which can be
    // less and can be nothing at all.
    let agreed: Vec<super::capability::Capability> = match accepting {
        Some(chosen) => token
            .requires()
            .into_iter()
            .filter(|capability| chosen.contains(capability))
            .collect(),
        None => token.requires(),
    };

    // This node's own identity, and the key the other end is about to
    // record. Neither is a grant — nothing has been given away yet.
    let keys = keys::ensure(database)
        .await
        .map_err(super::NetworkError::from)?;
    let me = super::ensure_self(database, config).await?;

    let accepted = call::announce(
        &token.endpoint,
        &token.secret,
        &Arriving {
            node: me.id.clone(),
            name: me.name.clone(),
            public_key: keys.public.clone(),
            accepted: Some(
                agreed
                    .iter()
                    .map(|capability| capability.name().to_string())
                    .collect(),
            ),
            // This node's own certificate authority, so the node being
            // joined can dial it back over the overlay and verify what
            // answers. Quiet if it cannot be read: a join that failed for
            // want of a field that only makes later calls faster would be
            // the wrong thing to refuse.
            ca: crate::edge::certs::ca_certificate_pem(database).await.ok(),
        },
    )
    .await?;

    if accepted.authority != token.authority {
        return Err(JoinError::NotWhoItClaimed {
            endpoint: token.endpoint,
            answered: accepted.authority,
            expected: token.authority,
        });
    }

    let authority = Node {
        usage: None,
        id: token.authority.clone(),
        name: token.name.clone(),
        // It has an address this node just reached, which is the whole
        // of what public means.
        kind: Kind::Public,
        endpoint: Some(token.endpoint.clone()),
        public_key: Some(token.public_key.clone()),
        overlay_ip: Some(token.overlay_ip.clone()),
        is_self: false,
        last_seen_at: Some(now_ms()),
        // What it said it would let this node ask of it, out of its own
        // mouth and in the token this node just used. Not a promise
        // this node can check — but it is the only source there is, and
        // a wrong one shows up as an errand that node refuses rather
        // than as anything silent.
        allows: token.offers(),
        ca_pem: None,
    };
    super::save(database, &authority).await?;

    // What this node has agreed the authority may ask of it. Written
    // before the grant, so there is never a moment where the authority
    // may send errands and nothing says which — an empty grant refuses
    // everything, which is a legible state, and the absence of rows
    // would have read as one too.
    super::capability::grant(database, &token.authority, &agreed)
        .await
        .map_err(super::NetworkError::from)?;

    // The row this whole thing exists to write. Revocable from here,
    // which is what makes joining not a one-way door.
    super::grant(database, &token.authority, &token.secret).await?;

    // The address the authority allocated, from the authority's answer
    // rather than from the token: the two agree, and if they ever did
    // not, the node that allocated it is the one that is right.
    let me = Node {
        overlay_ip: Some(accepted.overlay_ip),
        public_key: Some(keys.public.clone()),
        ..me
    };
    super::save(database, &me).await?;

    // The peer set just changed, so the interface has to. Not fatal:
    // the grant is written either way, and `doctor` reports an overlay
    // that did not come up.
    if let Err(error) = super::tunnel::ensure(database, config).await {
        tracing::warn!(%error, "joined, but the overlay did not come up");
    }

    tracing::info!(
        authority = %authority.id,
        name = %authority.name,
        overlay_ip = me.overlay_ip.as_deref().unwrap_or_default(),
        "joined a network"
    );

    Ok(Joined {
        authority,
        me,
        public_key: keys.public,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> JoinToken {
        JoinToken {
            authority: "nd-hub00000001".into(),
            name: "hub.example".into(),
            // Port 1 is reserved, so nothing on the machine running
            // this test is listening there either.
            endpoint: "127.0.0.1:1".into(),
            public_key: "0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into(),
            overlay_ip: "10.42.0.1".into(),
            assigned_ip: "10.42.0.2".into(),
            secret: "a-very-long-secret".into(),
            requires: None,
            offers: None,
        }
    }

    /// The property the order exists for: an authority this node could
    /// not reach is an authority it has not granted anything to.
    #[tokio::test]
    async fn an_authority_that_does_not_answer_is_granted_nothing() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = Config::default();

        let error = join(&database, &config, &token().encode(), None)
            .await
            .expect_err("nothing answered");
        assert!(matches!(error, JoinError::Call(_)), "{error}");

        assert!(
            super::super::authorities(&database)
                .await
                .expect("query")
                .is_empty(),
            "something was granted to a node that never answered"
        );
        assert_eq!(
            super::super::find(&database, &token().authority)
                .await
                .expect("query"),
            None
        );
    }

    /// Refused before anything opens a socket, and the message says
    /// which way it is wrong rather than reporting a failed connection
    /// to a host nobody meant to name.
    #[tokio::test]
    async fn something_that_is_not_a_token_never_reaches_the_network() {
        let database = crate::db::open_in_memory().await.expect("open");
        let error = join(&database, &Config::default(), "hunter2", None)
            .await
            .expect_err("not a token");

        assert!(matches!(error, JoinError::Token(_)), "{error}");
        assert!(error.to_string().contains("wdj1."), "{error}");
    }
}
