//! What this node is willing to do, for anybody — itself included.
//!
//! Distinct from a *grant*, which is what one node lets another ask of
//! it. This is one layer above: a capability the node does not provide
//! at all cannot be granted to anyone, and cannot be used by this node
//! on its own services either.
//!
//! That second half is the point. A small, cheap node can own projects
//! and services and run none of them, preferring to place every replica
//! somewhere with more room; and a node with a perfectly good public
//! address can decline to answer for names, which is a decision and not
//! a limitation.
//!
//! ## Private is a consequence, not a category
//!
//! `Kind::Private` used to mean "has no endpoint". It now means **does
//! not provide `Edge`**, which covers both the node that cannot and the
//! node that will not. The safety property is unchanged and it is the
//! one that matters: a node can only ever *reduce* what it claims.
//! Providing `Edge` requires an address the world can dial, so no
//! setting can make a node look reachable when it is not — which is why
//! `Kind` was derived from the endpoint in the first place.

use wabot::sqlite::{SqliteDatabase, SqliteResult};

/// A thing one node can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    /// Run replicas of a service, and pull the images for them.
    ///
    /// The registry pull comes with it rather than being a capability
    /// of its own: a node that may run your containers has to be able
    /// to fetch them, and a grant that cannot be used is not a smaller
    /// grant, it is a broken one.
    Host,
    /// Answer for a hostname, terminating TLS for it.
    Edge,
}

impl Capability {
    /// Where the answer lives in the `setting` table.
    fn key(&self) -> &'static str {
        match self {
            Capability::Host => "node.provides.host",
            Capability::Edge => "node.provides.edge",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Capability::Host => "host",
            Capability::Edge => "edge",
        }
    }

    pub const ALL: [Capability; 2] = [Capability::Host, Capability::Edge];
}

/// Whether this node offers a capability at all.
///
/// **Unset means yes.** Every node that exists today does both, and a
/// default of "no" would take a working network off the air to
/// introduce a setting. Turning one off is the deliberate act.
pub async fn provides(database: &SqliteDatabase, capability: Capability) -> bool {
    match crate::node::settings::read(database, capability.key()).await {
        Ok(Some(stored)) => stored != "off",
        Ok(None) => true,
        Err(error) => {
            // A database that cannot answer is not a reason to decide
            // the node does nothing. Reading as capable keeps a node
            // behaving the way it did a moment ago.
            tracing::warn!(%error, capability = capability.name(), "could not read a capability");
            true
        }
    }
}

pub async fn set_provides(
    database: &SqliteDatabase,
    capability: Capability,
    provides: bool,
) -> SqliteResult<()> {
    let value = match provides {
        true => "on",
        false => "off",
    };
    crate::node::settings::write(database, capability.key(), value).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_node_does_both_until_it_is_told_otherwise() {
        let database = crate::db::open_in_memory().await.expect("open");

        for capability in Capability::ALL {
            assert!(
                provides(&database, capability).await,
                "{} was off before anybody said so",
                capability.name()
            );
        }
    }

    /// Private is not a separate switch and must never become one: it
    /// is what a node *is* when it does not offer to answer for names.
    /// A node with a perfectly good address that declines is as private
    /// as one that never had an address at all.
    #[tokio::test]
    async fn a_node_that_declines_to_be_an_edge_is_private() {
        let database = crate::db::open_in_memory().await.expect("open");
        let config = crate::config::Config::default();
        crate::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("domain");

        let public = crate::network::ensure_self(&database, &config)
            .await
            .expect("self");
        assert!(
            public.may_be_edge(),
            "it has an address and offers to use it"
        );

        set_provides(&database, Capability::Edge, false)
            .await
            .expect("set");
        let private = crate::network::ensure_self(&database, &config)
            .await
            .expect("self");

        assert_eq!(private.kind, crate::network::Kind::Private);
        assert!(!private.may_be_edge());
        assert!(
            private.endpoint.is_none(),
            "an address it will not use must not be offered to anyone"
        );
        assert_eq!(private.id, public.id, "it is still the same node");
    }

    /// A node small enough to prefer hosting elsewhere runs nothing,
    /// including its own services. Which is why this is one setting and
    /// not a rule about other nodes.
    #[tokio::test]
    async fn a_capability_turned_off_stays_off() {
        let database = crate::db::open_in_memory().await.expect("open");

        set_provides(&database, Capability::Host, false)
            .await
            .expect("set");

        assert!(!provides(&database, Capability::Host).await);
        assert!(
            provides(&database, Capability::Edge).await,
            "the other one was not touched"
        );
    }
}
