//! Who else is on this network, as the node that enrolled them says.
//!
//! ## The star was never in the tunnel
//!
//! `tunnel::peers` builds a WireGuard peer for **every** row in the `node`
//! table that has a key and an address — it does not filter to authorities
//! or to nodes this one enrolled. The overlay is a star because of what the
//! rows *are*: a join writes exactly one on each side, so a member knows
//! its authority and nothing else.
//!
//! That has been paid for three times, each in the same shape: the
//! primary's address travelling in a standby's errand, the co-edges of a
//! hostname travelling in an edge errand, and an errand addressed to an
//! authority that nobody collected. Every phase distributed by hand what a
//! census gives once.
//!
//! And the authority already has it. Every report carries the reporting
//! node's endpoint, its capabilities, its usage and its own certificate
//! authority — so the node that enrolled everybody is the only one holding
//! the whole picture, and nothing asked it for it.
//!
//! ## Reachability, never consent
//!
//! What travels is what it takes to *reach* a machine: an address, a key,
//! an anchor, and where to dial. **Not `allows`.** That column is what a
//! node permits *the reader*, so the authority's copy says nothing about
//! what a member permits a sibling — and a roster that wrote it would make
//! `edge_candidates` offer nodes that will never collect the errand, which
//! is the fault the Alpine node had for an hour. A member learned from a
//! roster has an empty grant set, and that is what keeps the control plane
//! directed while the data plane meshes.
//!
//! Nor `usage`, which is a measurement and would make every roster look
//! changed; nor `kind`, which every writer *derives* from whether there is
//! an endpoint, and a kind on the wire is a kind that can arrive wrong.

use serde::{Deserialize, Serialize};
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::{Kind, NetworkResult};

/// What the authority answers with: its members, itself included.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Roster {
    pub members: Vec<Member>,
}

/// One member, and only what it takes to reach it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub node: String,
    /// For a log line and the console. Never a decision.
    pub name: String,
    /// `tunnel::peers` needs this and the key; `call::to_node` dials it.
    pub overlay_ip: String,
    pub public_key: String,
    /// Without it a call to that node cannot be verified, only encrypted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<String>,
    /// Where the world dials it, when it offers to be dialled. `None` is a
    /// private member — which is also how the receiving node derives its
    /// kind, rather than being told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
}

/// The members of the network this node is the authority of.
///
/// **The nodes this one enrolled, plus itself** — not every row in the
/// table. An authority may have joined somebody else, or learned a
/// co-edge, and handing those out would leak a third network into this
/// one. A node the operator forgot has no row and drops out on its own.
pub async fn of_this_node(database: &SqliteDatabase) -> NetworkResult<Roster> {
    let enrolled: Vec<String> = super::enrolment::all(database)
        .await?
        .into_iter()
        .filter_map(|enrolment| enrolment.used_by)
        .collect();
    let mine = crate::edge::certs::ca_certificate_pem(database).await.ok();

    let members = super::all(database)
        .await?
        .into_iter()
        .filter(|node| node.is_self || enrolled.contains(&node.id))
        .filter_map(|node| {
            let (Some(overlay_ip), Some(public_key)) =
                (node.overlay_ip.clone(), node.public_key.clone())
            else {
                // A row that has not finished joining. Skipped rather
                // than sent half-built: a member with no key is a peer
                // nobody can configure, and saying so would be inventing
                // a state the receiver has to handle.
                return None;
            };
            Some(Member {
                node: node.id.clone(),
                name: node.name.clone(),
                overlay_ip,
                public_key,
                // This node's own anchor is the local authority's public
                // half — `ensure_self` writes `ca_pem` as `None` for the
                // self row, because that column is what *others* sent.
                ca_pem: match node.is_self {
                    true => mine.clone(),
                    false => node.ca_pem.clone(),
                },
                endpoint: node.endpoint.clone(),
            })
        })
        .collect();
    Ok(Roster { members })
}

/// What applying a roster did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    /// Rows written. `tunnel::ensure` is worth calling only for these:
    /// otherwise this is a netlink read every fifteen seconds, and on a
    /// node whose kernel refuses the interface, a warning line with it.
    pub changed: usize,
    /// Members whose address is already held by a different node here.
    pub conflicts: Vec<(String, String)>,
}

/// Learn the members of a network, narrowly.
///
/// Not `learn_peer`: that one hardcodes `private` and no endpoint on
/// insert and never updates either, and a roster has to carry the endpoint
/// — it is what lets a private member dial a public one across sites.
///
/// Four rules, and each is a thing that would otherwise go wrong:
///
/// 1. **This node is not described to itself.** Nothing else may.
/// 2. **An authority is healed, never described**: only a missing anchor
///    is filled in. `collect` dials that endpoint on every pass, and on a
///    node with two authorities the roster of one would otherwise move the
///    address of the other — each allocates from its own accounting.
/// 3. **A collision is refused before it is attempted.** The unique index
///    on `overlay_ip` makes the write fail, and a caught-and-warned
///    failure is a peer that never works with nothing saying why.
/// 4. **Nothing is deleted.** A member withdrawn elsewhere stays a peer
///    here until somebody forgets it — and `AllowedIPs` is authorisation,
///    so a stale key keeps the right to claim its own `/32`. Said out loud
///    rather than tidied silently.
pub async fn apply(database: &SqliteDatabase, roster: &Roster) -> NetworkResult<Applied> {
    let mut applied = Applied::default();
    let me = super::me(database).await?.map(|me| me.id);
    let authorities: Vec<String> = super::authorities(database)
        .await?
        .into_iter()
        .map(|authority| authority.node_id)
        .collect();

    for member in &roster.members {
        if Some(&member.node) == me.as_ref() {
            continue;
        }
        if authorities.contains(&member.node) {
            if let Some(ca) = &member.ca_pem {
                if fill_missing_anchor(database, &member.node, ca).await? {
                    applied.changed += 1;
                }
            }
            continue;
        }
        match holder_of(database, &member.overlay_ip).await? {
            Some(held) if held != member.node => {
                applied
                    .conflicts
                    .push((member.node.clone(), member.overlay_ip.clone()));
                continue;
            }
            _ => {}
        }
        if remember(database, member).await? {
            applied.changed += 1;
        }
    }
    Ok(applied)
}

/// Which node holds this overlay address here, if any.
async fn holder_of(database: &SqliteDatabase, overlay_ip: &str) -> NetworkResult<Option<String>> {
    let overlay_ip = overlay_ip.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"id\" FROM node WHERE \"overlay_ip\" = ?1",
                    [overlay_ip],
                    |row| row.get(0),
                )
                .optional()
        })
        .await?)
}

/// Fill in an authority's anchor, and nothing else about it.
async fn fill_missing_anchor(
    database: &SqliteDatabase,
    node_id: &str,
    ca_pem: &str,
) -> NetworkResult<bool> {
    let (node_id, ca_pem) = (node_id.to_string(), ca_pem.to_string());
    Ok(database
        .write(move |connection| {
            let changed = connection.execute(
                "UPDATE node SET \"ca_pem\" = ?2 \
                 WHERE \"id\" = ?1 AND (\"ca_pem\" IS NULL OR \"ca_pem\" = '')",
                (node_id, ca_pem),
            )?;
            Ok(changed > 0)
        })
        .await?)
}

/// Write a member, touching only what a roster is allowed to say.
async fn remember(database: &SqliteDatabase, member: &Member) -> NetworkResult<bool> {
    let member = member.clone();
    // Derived here rather than carried: a kind on the wire is a kind that
    // can arrive wrong, and every other writer derives it from exactly
    // this.
    let kind = match member.endpoint.is_some() {
        true => Kind::Public,
        false => Kind::Private,
    };
    Ok(database
        .write(move |connection| {
            let changed = connection.execute(
                "INSERT INTO node \
                   (\"id\", \"name\", \"kind\", \"endpoint\", \"public_key\", \"overlay_ip\", \
                    \"is_self\", \"joined_at\", \"allows\", \"ca_pem\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7, '', ?8) \
                 ON CONFLICT (\"id\") DO UPDATE SET \
                   \"name\" = excluded.\"name\", \
                   \"kind\" = excluded.\"kind\", \
                   \"endpoint\" = excluded.\"endpoint\", \
                   \"public_key\" = excluded.\"public_key\", \
                   \"overlay_ip\" = excluded.\"overlay_ip\", \
                   \"ca_pem\" = COALESCE(excluded.\"ca_pem\", \"node\".\"ca_pem\") \
                 WHERE \"node\".\"name\" IS NOT excluded.\"name\" \
                    OR \"node\".\"kind\" IS NOT excluded.\"kind\" \
                    OR \"node\".\"endpoint\" IS NOT excluded.\"endpoint\" \
                    OR \"node\".\"public_key\" IS NOT excluded.\"public_key\" \
                    OR \"node\".\"overlay_ip\" IS NOT excluded.\"overlay_ip\" \
                    OR (\"node\".\"ca_pem\" IS NULL AND excluded.\"ca_pem\" IS NOT NULL)",
                (
                    member.node,
                    member.name,
                    kind.as_str(),
                    member.endpoint,
                    member.public_key,
                    member.overlay_ip,
                    super::now_ms(),
                    member.ca_pem,
                ),
            )?;
            Ok(changed > 0)
        })
        .await?)
}

/// Where a clash between two nodes' addresses is left for a person.
///
/// The `setting` store rather than a table of its own: it is one line, and
/// `node::settings` is `pub(crate)` for exactly this — a caller that owns
/// its key. The same shape `acme.last_error` has, and read by `doctor` and
/// the attention card for the same reason.
pub const CONFLICT: &str = "network.roster.conflict";

/// Record what could not be written, or clear it when nothing could not.
pub async fn record_conflicts(
    database: &SqliteDatabase,
    conflicts: &[(String, String)],
) -> NetworkResult<()> {
    let said = conflicts
        .iter()
        .map(|(node, address)| {
            format!("{node} was offered {address}, which another node here already holds")
        })
        .collect::<Vec<_>>()
        .join("; ");
    Ok(crate::node::settings::write(database, CONFLICT, &said).await?)
}

/// What the last pass could not write, if anything.
pub async fn conflicts(database: &SqliteDatabase) -> Option<String> {
    crate::node::settings::read(database, CONFLICT)
        .await
        .ok()
        .flatten()
        .filter(|said| !said.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// An authority with one node it enrolled, and one it did not.
    async fn network() -> SqliteDatabase {
        let database = crate::db::open_in_memory().await.expect("open");
        crate::node::settings::set_domain(&database, Some("hub.example"))
            .await
            .expect("domain");
        // A hub, not just a self row: an authority has a key and an
        // address, and without both it is not a peer anybody could
        // configure — which is what `of_this_node` filters on.
        super::super::ensure_hub(&database, &Config::default())
            .await
            .expect("self");

        for (id, name, address, key) in [
            ("nd-member01", "member", "10.42.0.7", "aaaa"),
            ("nd-stranger1", "stranger", "10.42.0.9", "bbbb"),
        ] {
            super::super::save(
                &database,
                &super::super::Node {
                    usage: None,
                    id: id.into(),
                    name: name.into(),
                    kind: Kind::Private,
                    endpoint: None,
                    public_key: Some(key.into()),
                    overlay_ip: Some(address.into()),
                    is_self: false,
                    last_seen_at: None,
                    allows: vec![super::super::capability::Capability::Host],
                    ca_pem: Some("anchor".into()),
                },
            )
            .await
            .expect("a node");
        }
        // Only the first was enrolled here; the second is a row this node
        // learned some other way.
        let admin = super::super::tests::admin(&database).await;
        let (enrolment, _) = super::super::enrolment::create(
            &database,
            "member",
            "10.42.0.7",
            &admin,
            &super::super::capability::Capability::ALL,
            &super::super::capability::Capability::ALL,
        )
        .await
        .expect("minted");
        super::super::enrolment::spend(&database, &enrolment.id, "nd-member01")
            .await
            .expect("spent");
        database
    }

    /// The members are the nodes this one enrolled, and itself.
    ///
    /// Not every row it holds: an authority may have joined somebody, or
    /// learned a co-edge, and handing those out leaks a third network into
    /// this one.
    #[tokio::test]
    async fn a_roster_lists_the_nodes_this_one_enrolled_and_itself() {
        let database = network().await;
        let roster = of_this_node(&database).await.expect("roster");

        let ids: Vec<String> = roster
            .members
            .iter()
            .map(|member| member.node.clone())
            .collect();
        assert!(ids.contains(&"nd-member01".to_string()), "{ids:?}");
        assert!(
            !ids.contains(&"nd-stranger1".to_string()),
            "a node this one did not enrol is not its network's business: {ids:?}"
        );
        // And itself, because a member needs its authority's anchor and
        // address like any other peer.
        let me = super::super::me(&database)
            .await
            .expect("query")
            .expect("self");
        assert!(ids.contains(&me.id), "{ids:?}");
    }

    /// **Reachability, never consent.**
    ///
    /// `allows` is what a node permits *the reader*, so the authority's
    /// copy says nothing about what a member permits a sibling. A roster
    /// that carried it would make `edge_candidates` offer nodes that never
    /// collect the errand — which is the fault the Alpine node had for an
    /// hour, silently. Asserted on the serialised form, which is where it
    /// cannot be re-broken by a refactor.
    #[tokio::test]
    async fn a_roster_never_says_what_a_node_allows() {
        let database = network().await;
        let roster = of_this_node(&database).await.expect("roster");
        let wire = serde_json::to_string(&roster).expect("serialise");

        assert!(!wire.contains("allows"), "{wire}");
        assert!(
            !wire.contains("usage"),
            "a measurement is not reachability: {wire}"
        );
        assert!(
            !wire.contains("\"kind\""),
            "a kind on the wire can arrive wrong: {wire}"
        );
    }

    /// Nothing describes this machine to this machine.
    #[tokio::test]
    async fn applying_a_roster_does_not_describe_this_node_to_itself() {
        let database = network().await;
        let me = super::super::me(&database)
            .await
            .expect("query")
            .expect("self");

        let applied = apply(
            &database,
            &Roster {
                members: vec![Member {
                    node: me.id.clone(),
                    name: "somebody elses idea".into(),
                    overlay_ip: "10.42.0.200".into(),
                    public_key: "zzzz".into(),
                    ca_pem: None,
                    endpoint: Some("elsewhere.example:443".into()),
                }],
            },
        )
        .await
        .expect("applied");

        assert_eq!(applied.changed, 0);
        let mine = super::super::me(&database)
            .await
            .expect("query")
            .expect("self");
        assert_eq!(mine.name, me.name);
        assert_eq!(mine.overlay_ip, me.overlay_ip);
    }

    /// An address another row already holds is refused, and said out loud.
    ///
    /// The unique index makes the write fail; catching that and warning is
    /// what the co-edge path did, and it leaves a peer that never works
    /// with nothing anywhere saying why. Two authorities allocate from
    /// their own accounting, so this is not a corner — it is what happens
    /// the first time a node joins a second network.
    #[tokio::test]
    async fn an_address_another_row_already_holds_is_refused_and_said_out_loud() {
        let database = network().await;

        let applied = apply(
            &database,
            &Roster {
                members: vec![Member {
                    node: "nd-elsewhere".into(),
                    name: "elsewhere".into(),
                    // Already `nd-member01`'s.
                    overlay_ip: "10.42.0.7".into(),
                    public_key: "cccc".into(),
                    ca_pem: None,
                    endpoint: None,
                }],
            },
        )
        .await
        .expect("applied");

        assert_eq!(applied.changed, 0);
        assert_eq!(applied.conflicts.len(), 1);
        assert!(super::super::find(&database, "nd-elsewhere")
            .await
            .expect("query")
            .is_none());
        // And the row that was there is untouched.
        let held = super::super::find(&database, "nd-member01")
            .await
            .expect("query")
            .expect("still there");
        assert_eq!(held.overlay_ip.as_deref(), Some("10.42.0.7"));

        record_conflicts(&database, &applied.conflicts)
            .await
            .expect("record");
        let said = conflicts(&database).await.expect("a line to read");
        assert!(said.contains("nd-elsewhere"), "{said}");
        assert!(said.contains("10.42.0.7"), "{said}");
    }

    /// An authority is healed, never described.
    ///
    /// Its endpoint is what `collect` dials on every pass, and on a node
    /// with two authorities the roster of one would otherwise move the
    /// address of the other — each allocates from its own accounting. Only
    /// a missing anchor is filled in, which is what cures a node that
    /// joined before the join answer carried one.
    #[tokio::test]
    async fn an_authority_is_only_ever_given_an_anchor_it_lacks() {
        let database = crate::db::open_in_memory().await.expect("open");
        super::super::save(
            &database,
            &super::super::Node {
                usage: None,
                id: "nd-hub00001".into(),
                name: "hub".into(),
                kind: Kind::Public,
                endpoint: Some("hub.example:443".into()),
                public_key: Some("hubkey".into()),
                overlay_ip: Some("10.42.0.1".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("the authority's row");
        super::super::grant(&database, "nd-hub00001", "the-secret")
            .await
            .expect("granted");

        let applied = apply(
            &database,
            &Roster {
                members: vec![Member {
                    node: "nd-hub00001".into(),
                    name: "renamed by somebody".into(),
                    overlay_ip: "10.42.0.99".into(),
                    public_key: "another".into(),
                    ca_pem: Some("the anchor".into()),
                    endpoint: Some("moved.example:443".into()),
                }],
            },
        )
        .await
        .expect("applied");

        assert_eq!(
            applied.changed, 1,
            "the anchor was missing and is now there"
        );
        let authority = super::super::find(&database, "nd-hub00001")
            .await
            .expect("query")
            .expect("still there");
        assert_eq!(authority.ca_pem.as_deref(), Some("the anchor"));
        assert_eq!(
            authority.endpoint.as_deref(),
            Some("hub.example:443"),
            "a roster must not move the endpoint this node dials"
        );
        assert_eq!(authority.overlay_ip.as_deref(), Some("10.42.0.1"));
        assert_eq!(authority.name, "hub");

        // And again changes nothing, so nothing rebuilds the interface.
        let again = apply(
            &database,
            &Roster {
                members: vec![Member {
                    node: "nd-hub00001".into(),
                    name: "hub".into(),
                    overlay_ip: "10.42.0.1".into(),
                    public_key: "hubkey".into(),
                    ca_pem: Some("the anchor".into()),
                    endpoint: Some("hub.example:443".into()),
                }],
            },
        )
        .await
        .expect("applied");
        assert_eq!(again.changed, 0);
    }

    /// A census that says nothing new leaves the kernel alone.
    ///
    /// `tunnel::ensure` is called only when a row moved. Every fifteen
    /// seconds otherwise is a netlink read, and on a node whose kernel
    /// refuses the interface it is a warning line at the same rate.
    #[tokio::test]
    async fn a_roster_that_says_nothing_new_changes_nothing() {
        let database = network().await;
        let roster = Roster {
            members: vec![Member {
                node: "nd-fresh0001".into(),
                name: "fresh".into(),
                overlay_ip: "10.42.0.30".into(),
                public_key: "dddd".into(),
                ca_pem: Some("anchor".into()),
                endpoint: None,
            }],
        };

        assert_eq!(apply(&database, &roster).await.expect("first").changed, 1);
        assert_eq!(
            apply(&database, &roster).await.expect("second").changed,
            0,
            "the same census twice is one write"
        );

        // What it learned is a peer: an address, a key and an anchor.
        let learned = super::super::find(&database, "nd-fresh0001")
            .await
            .expect("query")
            .expect("learned");
        assert_eq!(learned.overlay_ip.as_deref(), Some("10.42.0.30"));
        assert!(learned.public_key.is_some());
        assert!(learned.ca_pem.is_some());
        assert_eq!(learned.kind, Kind::Private, "no endpoint means private");
        assert!(
            learned.allows.is_empty(),
            "a roster carries no consent, so a learned member may be asked for nothing"
        );
    }
}
