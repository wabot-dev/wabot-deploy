//! A replica: one copy of a service, and the node it runs on.
//!
//! **A replica is the unit.** A service is *n* placements, and a slot
//! number belongs to the service rather than the node it lands on.

use serde::Serialize;

/// One placement of a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replica {
    pub id: String,
    pub service_id: String,
    /// Which node runs it. `None` is this one — see migration `0020`.
    pub node_id: Option<String>,
    /// Its number within the service, from 1.
    pub slot: u32,
    pub address: Option<String>,
    /// The port on **this node's overlay address** that reaches this
    /// copy, for an edge running somewhere else. `None` until something
    /// outside the node needs to reach it. See migration `0022`.
    pub overlay_port: Option<u16>,
    pub last_error: Option<String>,
    /// Set when the node running it threw it out. The node that placed
    /// it stops asking.
    pub evicted_at: Option<i64>,
    /// The last octet of the address this copy keeps on its project's
    /// bridge, for as long as the copy exists.
    ///
    /// `None` is every copy that does not need one — which is all of
    /// them except a database's. See migration `0030`: a connection
    /// string is written down somewhere, so the address behind it
    /// cannot move when the container is recreated, and `host-local`
    /// hands out the lowest free one.
    pub reserved_host: Option<u8>,
    /// What it was using when the node running it last said so.
    ///
    /// The one field on this row that is a **measurement** rather than a
    /// decision, which is why it is deliberately not part of what makes a
    /// report count as news — see migration `0034`. A copy here is read
    /// from its cgroup instead; this is for the ones that are not.
    pub memory_bytes: Option<u64>,
    /// What its volume held when that node last said so.
    ///
    /// The figure that matters most for a database and the one nothing
    /// else watches: memory is a ceiling somebody chose, and a volume
    /// grows until the machine is full. A measurement like the one above,
    /// with the same rule — see migration `0035`.
    pub disk_bytes: Option<u64>,
    /// What it was using when the node running it last worked it out, in
    /// millicores — a thousand is one core.
    ///
    /// The rate is computed **there**: a counter crossing the network
    /// would leave this node dividing by a gap it can only guess at. See
    /// migration `0036`, which also explains why this number is smoother
    /// than the one for a copy here.
    pub cpu_millicores: Option<u32>,
    /// Whether this read-only copy is still following its primary.
    ///
    /// `None` is **not asked yet**, and it has to stay distinguishable:
    /// a node that has just started has no opinion, and rendering "not
    /// following" for a copy nobody has asked about is a page inventing
    /// an outage. See `migrations/0039_following.sql`.
    pub following: Option<bool>,
    /// Write-ahead log the primary is holding for this copy's slot, in
    /// bytes. The consequence of not following, rather than the symptom.
    pub wal_held: Option<u64>,
    /// Who put this copy on this node, and on what grounds.
    ///
    /// `Some(Placed::Planner)` with a sentence beside it is the
    /// scheduler's; anything else is a person's, and **a person's is
    /// never moved by the scheduler**. See migration `0052`.
    pub placed_by: Option<Placed>,
    pub placed_note: Option<String>,
}

/// Who decided where a copy runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placed {
    /// Somebody named this node. Never moved automatically.
    Person,
    /// The scheduler chose it, and `placed_note` says why.
    Planner,
}

impl Placed {
    pub fn as_str(self) -> &'static str {
        match self {
            Placed::Person => "person",
            Placed::Planner => "planner",
        }
    }

    /// Unknown reads as a person's, which is the safer of the two: the
    /// scheduler leaves it alone. A row written by a newer node naming a
    /// decider this one has never heard of must not become something
    /// this version feels free to move.
    pub fn parse(text: &str) -> Self {
        match text {
            "planner" => Placed::Planner,
            _ => Placed::Person,
        }
    }
}

impl Replica {
    /// Whether this copy runs on the node reading the row.
    pub fn is_here(&self) -> bool {
        self.node_id.is_none()
    }

    pub fn evicted(&self) -> bool {
        self.evicted_at.is_some()
    }

    /// The containerd container id for this copy.
    ///
    /// **Slot 1 keeps the name the service always had.** A container id
    /// is what reconciliation matches a running container against, so
    /// renaming every existing one would make an upgrade start a second
    /// copy of everything and leave the first running under a name
    /// nothing looks for any more. The suffix appears from slot 2, on
    /// containers that did not exist before this did.
    pub fn container_id(&self, project_slug: &str, service_slug: &str) -> String {
        container_id_for(project_slug, service_slug, self.slot)
    }
}

/// The same id, from a slot number rather than a row.
///
/// For a caller that has the slot and not the replica — the log stream,
/// which is handed one in a query string and must build the same id the
/// deploy did. One function so the two cannot drift: the rule that slot
/// 1 keeps the old name is the sort that gets reimplemented slightly
/// differently and then costs an afternoon.
pub fn container_id_for(project_slug: &str, service_slug: &str, slot: u32) -> String {
    match slot {
        1 => format!("{project_slug}.{service_slug}"),
        slot => format!("{project_slug}.{service_slug}.{slot}"),
    }
}
