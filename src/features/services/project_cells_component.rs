//! One row of a project's live table, formatted.
//!
//! **Built here rather than in the browser** so the two halves cannot
//! disagree: the same values render the first paint and every update
//! after it. Same reason as [`super::service_state_component::StateCell`].
//!
//! They lived in `console/projects.rs` and were imported *out* of it by
//! three view components — the dependency pointing the wrong way, which
//! is what splitting that file into its two controllers made
//! unavoidable to notice. Nothing here reads the database; what does is
//! `console::live`.

/// One replica, as its row reads.
#[derive(serde::Serialize)]
pub struct ReplicaCell {
    pub(crate) badge: &'static str,
    pub(crate) dot: &'static str,
    pub(crate) word: String,
    /// What this copy is using, in millicores — a thousand is one core.
    ///
    /// Only for the copies here, and only from the second tick: a rate is
    /// the difference between two readings, which is why it lives on a
    /// stream rather than on a page render. Empty is "no answer yet", and
    /// the page leaves what it had rather than writing a dash over a
    /// figure.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) cpu: String,
    /// The line under it: an address while it is up, a reason while it
    /// is not, empty when there is nothing to add.
    pub(crate) detail: String,
}

/// One hostname, as far as its certificate has got.
#[derive(serde::Serialize)]
pub struct NameCell {
    /// Whether to show the "on the way" badge at all. The page renders
    /// it either way and this hides it — the same mechanism the deploy
    /// and stop controls use, so nothing has to be built client-side.
    pub waiting: bool,
}

/// One node asked to serve one name, as far as the asking has got.
#[derive(serde::Serialize)]
pub struct EdgeCell {
    pub(crate) badge: &'static str,
    pub(crate) dot: &'static str,
    /// Owned, because it is translated on the way out and the Spanish for
    /// a constant is a different constant. One allocation per edge per
    /// tick, against a second spelling of `language::word` for static
    /// text — which is the kind of pair that drifts.
    pub(crate) word: String,
}

/// What every replica of every service in this project is doing.
/// How far the instruction to one node has got.
///
/// **Shared with the page's own render**, which is the point. The edges
/// card wrote `Asked` into its markup whenever a box was ticked and left
/// the stream to correct it — so a page the stream does not reach said
/// `Asked` for ever. Jorge's node said it about a name it was already
/// serving with a certificate, and its errand table was empty: with
/// nothing pending, the answer was always "serving" and nothing computed
/// it.
///
/// The server knows this when it renders.
/// `the_first_paint_and_the_stream_agree` is a test on the node page for
/// exactly this shape; this card did not have it.
pub fn edge_cell(latest: Option<&crate::infra::network::errand::Record>) -> EdgeCell {
    match latest {
        Some(order) if order.error.is_some() => EdgeCell {
            badge: "badge badge-danger",
            dot: "dot dot-danger",
            word: "Refused".into(),
        },
        Some(order) if order.done() => EdgeCell {
            badge: "badge badge-success",
            dot: "dot dot-success",
            word: "Serving".into(),
        },
        Some(_) => EdgeCell {
            badge: "badge badge-info",
            dot: "dot dot-info dot-pulse",
            word: "Asked".into(),
        },
        // Nothing pending and nothing refused: this node answers for the
        // name. A single-node install has never had an errand in its
        // life, and this is the branch it lives in.
        None => EdgeCell {
            badge: "badge badge-success",
            dot: "dot dot-success",
            word: "Serving".into(),
        },
    }
}

/// What one replica's row says — the **only** decision, read by the first
/// paint and by every update after it.
///
/// The comment here used to say the words were "kept beside"
/// `placement_state` deliberately, and name the failure that would cause:
/// a page saying one thing on load and another two seconds later. It then
/// happened exactly as described. A stopped service's remote copy was
/// given its own word in `placement_state` and not here, so the first
/// paint said "not running" and the stream put "waiting for that node"
/// back over it — reported by Jorge with the two rows beside each other,
/// the local copies right and the remote one wrong.
///
/// So `placement_state` renders *this*, and there is nothing left to keep
/// in step.
pub fn replica_cell(
    replica: &crate::domain::entities::replica::Replica,
    // Whether the service is meant to be running at all. Nobody waits for
    // news about a copy that is not, here or anywhere else.
    stopped: bool,
    // Whether an instruction about it is still waiting to be collected,
    // which is a different thing from silence.
    queued: bool,
    // Whether the edge has taken it out of the rotation: its container
    // is up and nothing answers on the port. A distinct state from every
    // other one here, and the one somebody is looking for when a service
    // is "up" and losing requests.
    not_answering: bool,
) -> ReplicaCell {
    if replica.evicted() {
        return ReplicaCell {
            badge: "badge badge-warning",
            dot: "",
            word: "Evicted there".into(),
            cpu: String::new(),
            detail: String::new(),
        };
    }
    if let Some(failure) = &replica.last_error {
        return ReplicaCell {
            badge: "badge badge-danger",
            dot: "dot dot-danger",
            word: "Failed".into(),
            cpu: String::new(),
            detail: failure.clone(),
        };
    }
    // Running, and no longer replicating. **Above the rotation check and
    // below the failures**, which is where it belongs: nothing failed —
    // the container is up and answering — and what is wrong is that its
    // data stopped moving. A copy somebody reads from that is silently
    // hours out of date is worse than one that is down, because down is
    // visible.
    //
    // Only ever `Some(false)`: `None` is "not asked yet", and rendering
    // this for a copy nobody has asked about would be the page inventing
    // an outage.
    if replica.following == Some(false) && !replica.evicted() {
        return ReplicaCell {
            badge: "badge badge-danger",
            dot: "dot dot-danger",
            word: "Not following".into(),
            cpu: String::new(),
            detail: match replica.wal_held {
                Some(held) if held > 0 => format!(
                    "{} of write-ahead log held for it",
                    crate::infra::node::memory::human(held)
                ),
                _ => String::new(),
            },
        };
    }
    if let Some(address) = &replica.address {
        // Running and out of the rotation. Not `Failed`, which is what
        // the *node* said about starting it, and not `Running`, which
        // would be the page agreeing with a container that is lying: the
        // process is there and nothing answers on the port it was given.
        // Warning rather than danger, because the edge has already moved
        // the traffic — this is somebody to tell, not an outage.
        if not_answering {
            return ReplicaCell {
                badge: "badge badge-warning",
                dot: "dot dot-warning",
                word: "Not answering".into(),
                cpu: String::new(),
                detail: address.clone(),
            };
        }
        return ReplicaCell {
            badge: "badge badge-success",
            dot: "dot dot-success",
            word: "Running".into(),
            cpu: String::new(),
            detail: address.clone(),
        };
    }
    if replica.is_here() || stopped {
        return ReplicaCell {
            badge: "badge",
            dot: "",
            word: "Not running".into(),
            cpu: String::new(),
            detail: String::new(),
        };
    }
    match queued {
        // Written and not yet collected, which is most of what makes a
        // wait feel long.
        true => ReplicaCell {
            badge: "badge badge-info",
            dot: "dot dot-info dot-pulse",
            word: "Queued for that node".into(),
            cpu: String::new(),
            detail: String::new(),
        },
        // Told, and nothing has come back. The node reports when it
        // collects, so this is the honest word until it does.
        false => ReplicaCell {
            badge: "badge badge-info",
            dot: "dot dot-info dot-pulse",
            word: "Waiting for that node".into(),
            cpu: String::new(),
            detail: String::new(),
        },
    }
}
