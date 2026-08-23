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
//! ## And one switch that is not a capability
//!
//! [`owns`] says whether this node originates any work of its own. It
//! lives here because it answers the same question — what is this node
//! for — and it is deliberately **outside** [`Capability`]: see its own
//! doc for why that boundary matters.
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
    /// Hold a copy of somebody's **data**: a database's standby, and the
    /// volume it lives in.
    ///
    /// Its own capability rather than part of `Host`, because running
    /// somebody's container and keeping somebody's data are different
    /// things to agree to. A node with room to spare and a disk it does
    /// not want filled is an ordinary thing to be, and under one
    /// capability there would be no way to say so.
    ///
    /// **Not backfilled onto existing joins.** A node that agreed to run
    /// containers last month did not agree to keep a copy of a database,
    /// and a migration that decided otherwise would be the wrong kind of
    /// quiet. Granting it needs a re-join.
    Store,
    /// Keep this node **up to date**, and ask it to restart or back
    /// itself up.
    ///
    /// The one capability that is not about somebody else's work. `Host`,
    /// `Edge` and `Store` all mean "carry this thing of mine"; this one
    /// means "look after the machine". So it is the one a node grants to
    /// the console it is administered from, and the reason there can be
    /// such a console at all.
    ///
    /// Bounded on purpose, and the bound is what makes it grantable: it
    /// installs a release *this project publishes*, restarts the service,
    /// and takes a backup. It reads no data, creates nothing, and touches
    /// no account. A capability that meant "be the operator here" would
    /// not be revocable in any useful sense — an operator can write
    /// itself a way back in — and nothing here needs one.
    ///
    /// **Not backfilled**, for the reason `Store` is not.
    Manage,
}

impl Capability {
    /// Where the answer lives in the `setting` table.
    fn key(&self) -> &'static str {
        match self {
            Capability::Host => "node.provides.host",
            Capability::Edge => "node.provides.edge",
            Capability::Store => "node.provides.store",
            Capability::Manage => "node.provides.manage",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Capability::Host => "host",
            Capability::Edge => "edge",
            Capability::Store => "store",
            Capability::Manage => "manage",
        }
    }

    /// Whether a node's address is what makes this possible.
    ///
    /// Only `Edge` needs one, and the console disables the switch
    /// without it: offering to answer for names with nothing the world
    /// can dial is a promise the node cannot keep. Asked here rather
    /// than written into the form, so a capability added later cannot
    /// arrive without an answer.
    pub fn needs_endpoint(&self) -> bool {
        matches!(self, Capability::Edge)
    }

    pub const ALL: [Capability; 4] = [
        Capability::Host,
        Capability::Edge,
        Capability::Store,
        Capability::Manage,
    ];
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

/// Whether this node is the **owner** of anything.
///
/// Projects, services, the hostnames they answer for — the work that
/// produces errands rather than the work that carries them out. Off, and
/// this node is somewhere resources live: it runs what it was asked to
/// run, answers for names it was told to answer for, keeps the copies it
/// agreed to keep, and originates nothing.
///
/// **Not a [`Capability`], and it must never become one.** Every variant
/// of that enum is a thing *another* node asks of this one, which is what
/// makes a grant, a token term and a consent check mean anything. This is
/// a decision about the node itself that nobody asks for, so it has no
/// grant, appears in no token and belongs on no terms screen — where it
/// would be a sentence with no second party in it.
///
/// Unset means yes, like the capabilities beside it and for the same
/// reason: every node that exists today owns its own work.
pub async fn owns(database: &SqliteDatabase) -> bool {
    match crate::node::settings::read(database, OWNS).await {
        Ok(Some(stored)) => stored != "off",
        Ok(None) => true,
        // Reading as capable keeps a node behaving the way it did a
        // moment ago — the same fallback `provides` makes.
        Err(error) => {
            tracing::warn!(%error, "could not read whether this node owns anything");
            true
        }
    }
}

pub async fn set_owns(database: &SqliteDatabase, owns: bool) -> SqliteResult<()> {
    let value = match owns {
        true => "on",
        false => "off",
    };
    crate::node::settings::write(database, OWNS, value).await
}

/// In the same namespace as a capability's key, because it is the same
/// question about the node — and not in [`Capability::key`], because
/// nothing may grant it.
const OWNS: &str = "node.provides.own";

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

/// The operator's own answer, for one node, from this node's console.
///
/// [`grant`] is what a *join* writes: the terms that were agreed when the
/// two nodes met. This is somebody changing their mind afterwards, which
/// needs to say two things rather than one — what is granted now, and
/// what has been **refused**.
///
/// The refusal has to be a row of its own. `network::honour_offers` runs
/// at every boot and re-adds whatever the enrolment offered, asking about
/// the enrolment and not about the history — so the absence of a grant is
/// not a decision it can see, and a withdrawal expressed that way would
/// be undone by the next restart. Its own doc predicted exactly this.
pub async fn set_granted(
    database: &SqliteDatabase,
    node_id: &str,
    capabilities: &[Capability],
) -> SqliteResult<()> {
    // Derived from the enum rather than listed, so a capability added
    // later cannot be quietly unrefusable.
    let refused: Vec<String> = Capability::ALL
        .into_iter()
        .filter(|capability| !capabilities.contains(capability))
        .map(|capability| capability.name().to_string())
        .collect();

    let node = node_id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM node_grant_withdrawn WHERE \"node_id\" = ?1",
                [&node],
            )?;
            for capability in &refused {
                connection.execute(
                    "INSERT INTO node_grant_withdrawn (\"node_id\", \"capability\", \"withdrawn_at\") \
                     VALUES (?1, ?2, ?3)",
                    (&node, capability, crate::platform::now_ms()),
                )?;
            }
            Ok(())
        })
        .await?;

    grant(database, node_id, capabilities).await
}

/// What the operator has refused `node_id`, whatever a join agreed.
pub async fn withdrawn_from(database: &SqliteDatabase, node_id: &str) -> Vec<Capability> {
    let node_id = node_id.to_string();
    let names: Vec<String> = database
        .read(move |connection| {
            connection
                .prepare("SELECT \"capability\" FROM node_grant_withdrawn WHERE \"node_id\" = ?1")?
                .query_map([node_id], |row| row.get(0))?
                .collect()
        })
        .await
        .unwrap_or_default();
    names.iter().filter_map(|name| parse_one(name)).collect()
}

/// What this node has agreed to do for `node_id`.
pub async fn granted_to(database: &SqliteDatabase, node_id: &str) -> Vec<Capability> {
    let node_id_for_withdrawals = node_id.to_string();
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
    //
    // And through what the operator has withdrawn for this node in
    // particular. `honour_offers` re-adds whatever the enrolment offered
    // at every boot, so without this a withdrawal would last until the
    // next restart. Belt as well as braces — that pass skips a withdrawn
    // capability too — because any future path that grants blindly has to
    // be bounded by the more recent decision rather than trusted to
    // remember it.
    let withdrawn = withdrawn_from(database, &node_id_for_withdrawals).await;
    let mut held = Vec::new();
    for capability in Capability::ALL {
        if names.iter().any(|name| name == capability.name())
            && !withdrawn.contains(&capability)
            && provides(database, capability).await
        {
            held.push(capability);
        }
    }
    held
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
        .filter_map(parse_one)
        .collect()
}

/// One name, or nothing if this version has never heard of it.
pub fn parse_one(name: &str) -> Option<Capability> {
    Capability::ALL
        .into_iter()
        .find(|capability| capability.name() == name)
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
        assert!(
            !granted_to(&database, "nd-them")
                .await
                .contains(&Capability::Host),
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

        // Everything that was granted except the one withdrawn — said
        // that way rather than as a literal list, so adding a fourth
        // capability does not make this test wrong instead of the code.
        let expected: Vec<Capability> = Capability::ALL
            .into_iter()
            .filter(|capability| *capability != Capability::Host)
            .collect();
        assert_eq!(granted_to(&database, "nd-them").await, expected);
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

    /// The withdrawal has to be a row of its own.
    ///
    /// `honour_offers` runs at every boot and re-adds whatever the
    /// enrolment offered, because it asks about the enrolment rather than
    /// about the history — so a withdrawal expressed by deleting a grant
    /// would last until the next restart, and the operator would have no
    /// way to tell that from it never having worked at all.
    #[tokio::test]
    async fn a_withdrawn_grant_is_not_restored_by_granting_again() {
        let database = crate::db::open_in_memory().await.expect("open");
        grant(&database, "nd-them", &Capability::ALL)
            .await
            .expect("grant");

        // The operator changes their mind about one of them.
        let kept: Vec<Capability> = Capability::ALL
            .into_iter()
            .filter(|capability| *capability != Capability::Manage)
            .collect();
        set_granted(&database, "nd-them", &kept).await.expect("set");
        assert_eq!(granted_to(&database, "nd-them").await, kept);

        // And something grants the whole set again — which is what the
        // boot pass does with whatever the join offered.
        grant(&database, "nd-them", &Capability::ALL)
            .await
            .expect("grant");
        assert!(
            !granted_to(&database, "nd-them")
                .await
                .contains(&Capability::Manage),
            "the more recent decision has to win against a pass that asks about the join"
        );
    }

    /// The repair for what the form wrote in silence.
    ///
    /// `store` has never had a checkbox, so every stored `off` for it was
    /// the handler writing a setting the card did not draw — a decision
    /// nobody could have made, on a node that then refused the database
    /// errands for copies it was already holding. Migration `0050` deletes
    /// exactly that value and nothing else.
    #[tokio::test]
    async fn a_setting_nobody_could_have_chosen_is_undone() {
        let database = crate::db::open_in_memory().await.expect("open");

        // What the old form left behind, written the way it wrote it.
        crate::node::settings::write(&database, "node.provides.store", "off")
            .await
            .expect("as the bug did");
        // And a decision somebody *could* have made, which must survive.
        set_provides(&database, Capability::Host, false)
            .await
            .expect("set");

        // The statement itself, not the runner: `open_in_memory` has
        // already applied every migration, and one that has run is not
        // meant to run again. What has the risk in it is the `WHERE` —
        // that it takes this value and leaves every other decision.
        database
            .write(|connection| {
                connection
                    .execute_batch(include_str!("../../migrations/0050_undo_store_off.sql"))?;
                Ok(())
            })
            .await
            .expect("the repair");

        assert!(
            provides(&database, Capability::Store).await,
            "a value no form could write is not an answer to keep"
        );
        assert!(
            !provides(&database, Capability::Host).await,
            "and one that could is left exactly alone"
        );
    }

    /// Owning things is not something another node asks for, so it must
    /// never be in the enum that says what one node may ask of another.
    /// Putting it there would offer it on the terms screen of a join,
    /// where it would be a sentence with no second party in it.
    #[test]
    fn owning_is_not_a_capability_anybody_can_be_granted() {
        assert!(
            !Capability::ALL
                .iter()
                .any(|capability| capability.name() == "own"),
            "the ownership switch has to stay outside the grantable set"
        );
        assert!(parse_one("own").is_none(), "and outside a token's terms");
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
