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

/// Let `node_id` ask this node to do these things, and nothing else.
///
/// The whole set each time, not an addition: what arrives is the terms
/// somebody agreed to, and a grant that could only ever grow would make
/// "I accepted less this time" impossible to say.
///
/// Bounded by what this node provides at all. Granting a capability it
/// has turned off would be a promise it has already decided not to keep,
/// and the node asking would find out from an errand that never worked
/// rather than from anything that said so.
pub async fn grant(
    database: &SqliteDatabase,
    node_id: &str,
    capabilities: &[Capability],
) -> SqliteResult<()> {
    let mut allowed = Vec::new();
    for capability in capabilities {
        if provides(database, *capability).await {
            allowed.push(capability.name().to_string());
        }
    }

    let node_id = node_id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM node_grant WHERE \"node_id\" = ?1", [&node_id])?;
            for capability in &allowed {
                connection.execute(
                    "INSERT INTO node_grant (\"node_id\", \"capability\", \"granted_at\") \
                     VALUES (?1, ?2, ?3)",
                    (&node_id, capability, crate::platform::now_ms()),
                )?;
            }
            Ok(())
        })
        .await
}

/// What this node has agreed to do for `node_id`.
pub async fn granted_to(database: &SqliteDatabase, node_id: &str) -> Vec<Capability> {
    let node_id = node_id.to_string();
    let names: Vec<String> = database
        .read(move |connection| {
            connection
                .prepare("SELECT \"capability\" FROM node_grant WHERE \"node_id\" = ?1")?
                .query_map([node_id], |row| row.get(0))?
                .collect()
        })
        .await
        .unwrap_or_default();

    // Filtered through what this node provides *now*: a grant written
    // before a switch was turned off is a promise it can no longer
    // keep, and the switch is the more recent decision.
    let mut held = Vec::new();
    for capability in Capability::ALL {
        if names.iter().any(|name| name == capability.name())
            && provides(database, capability).await
        {
            held.push(capability);
        }
    }
    held
}

/// Every node this one may send `capability` to.
///
/// The question both selectors ask, and the one they were getting wrong:
/// a node that never agreed to run your containers is not somewhere you
/// can place a replica, and offering it produces an errand nobody will
/// ever collect.
pub async fn may_be_asked(database: &SqliteDatabase, capability: Capability) -> Vec<String> {
    let name = capability.name().to_string();
    database
        .read(move |connection| {
            connection
                .prepare("SELECT \"node_id\" FROM node_grant WHERE \"capability\" = ?1")?
                .query_map([name], |row| row.get(0))?
                .collect()
        })
        .await
        .unwrap_or_default()
}

/// Read a comma-separated list back, dropping anything this version does
/// not know.
///
/// An unknown capability from a newer node is left out rather than
/// refused: the rest of the terms are still readable, and a join that
/// failed entirely because one word was new would make every upgrade a
/// flag day.
pub fn parse_list(text: &str) -> Vec<Capability> {
    text.split(',')
        .map(str::trim)
        .filter_map(|name| {
            Capability::ALL
                .into_iter()
                .find(|capability| capability.name() == name)
        })
        .collect()
}

pub fn to_list(capabilities: &[Capability]) -> String {
    capabilities
        .iter()
        .map(|capability| capability.name())
        .collect::<Vec<_>>()
        .join(",")
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

    /// The whole point of the terms screen. A node may agree to serve
    /// somebody's names and never run their containers, and until
    /// grants existed there was no way to say so — joining granted
    /// everything, for ever, because there had only ever been one kind
    /// of errand.
    #[tokio::test]
    async fn a_node_can_agree_to_one_thing_and_refuse_the_other() {
        let database = crate::db::open_in_memory().await.expect("open");

        grant(&database, "nd-them", &[Capability::Edge])
            .await
            .expect("grant");

        assert_eq!(
            granted_to(&database, "nd-them").await,
            vec![Capability::Edge]
        );
        assert_eq!(may_be_asked(&database, Capability::Edge).await, ["nd-them"]);
        assert!(
            may_be_asked(&database, Capability::Host).await.is_empty(),
            "it never agreed to run anything"
        );
    }

    /// Granting is the whole set, not an addition. Terms are agreed
    /// again each time, and a grant that could only ever grow would
    /// make "I accepted less this time" impossible to say.
    #[tokio::test]
    async fn agreeing_again_replaces_what_was_agreed_before() {
        let database = crate::db::open_in_memory().await.expect("open");

        grant(&database, "nd-them", &Capability::ALL)
            .await
            .expect("grant");
        grant(&database, "nd-them", &[Capability::Host])
            .await
            .expect("grant");

        assert_eq!(
            granted_to(&database, "nd-them").await,
            vec![Capability::Host]
        );
    }

    /// A switch is the more recent decision. A grant written before one
    /// was turned off is a promise this node can no longer keep, and
    /// honouring it would have the console offering something the
    /// operator has already declined.
    #[tokio::test]
    async fn turning_a_capability_off_withdraws_what_was_granted_of_it() {
        let database = crate::db::open_in_memory().await.expect("open");
        grant(&database, "nd-them", &Capability::ALL)
            .await
            .expect("grant");

        set_provides(&database, Capability::Host, false)
            .await
            .expect("set");

        assert_eq!(
            granted_to(&database, "nd-them").await,
            vec![Capability::Edge]
        );
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
