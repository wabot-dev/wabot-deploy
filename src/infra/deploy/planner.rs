//! Where a copy goes when nobody said.
//!
//! ## Deliberately dumb
//!
//! Spread the copies of one service across distinct machines, respect
//! the grant, respect what each has left to promise. That is most of the
//! value and it fits in a sentence, which is what makes an operator
//! willing to run it — a scheduler nobody can explain is one nobody
//! turns on.
//!
//! What it is **not**: bin packing, affinity, topology, preemption. Each
//! of those trades an explanation for a percentage, and this platform's
//! whole claim is a box you own and can reason about.
//!
//! ## A placement is still a row
//!
//! Whatever this decides, the result is the same `replica` a person
//! could have written by hand, carrying `placed_by` and one line saying
//! why. Two consequences, and they are the rules rather than
//! conveniences:
//!
//! - **A row a person wrote is never moved by this.** Not weighed
//!   against a better fit, not rebalanced. Somebody pinned it.
//! - **It places what is not placed; it never moves what runs.** Moving
//!   is draining, and draining is something a person asks for. A
//!   scheduler that shuffles running containers to improve a number is
//!   one that causes outages nobody ordered.
//!
//! ## Not choosing is not the same as not refusing
//!
//! `fits` declines to enforce a rule made of a missing number: a machine
//! whose meters cannot be read does not get to refuse anybody. The
//! selector here answers the other question and answers it the other
//! way — a node that has not said what it has is **not a candidate**,
//! because sending work to a machine you know nothing about is guessing
//! rather than deciding. Somebody may still name it by hand.

use super::Rooms;

/// One machine this copy could go on, and what is known about it.
#[derive(Debug, Clone)]
pub struct Candidate {
    /// `None` is this node — the same spelling `replica.node_id` uses,
    /// so what comes out of here is what goes into the row.
    pub node: Option<String>,
    /// For the sentence on the row. Never a decision.
    pub name: String,
    pub rooms: Rooms,
    /// Copies of **this** service it already holds. The spread.
    pub of_this_service: u32,
    /// Copies of anything, which is the blast radius if it dies.
    pub replicas: u32,
    /// Copies it holds with no ceiling, so its arithmetic is a floor.
    pub unbounded: u32,
    /// Whether a copy of this service was thrown off here.
    ///
    /// Somebody on that machine did that deliberately, and putting one
    /// back is asking them again. Never a candidate; still nameable by
    /// hand, which is the door that stays open.
    pub evicted_here: bool,
    /// Whether somebody is taking this machine out of service.
    ///
    /// Never a candidate, for the plainest reason on this list: putting
    /// a copy on a node somebody is emptying means the drain has to move
    /// it again, and a drain that keeps finding new work is one that
    /// never finishes.
    pub draining: bool,
}

/// What the copy costs, per copy.
#[derive(Debug, Clone, Copy, Default)]
pub struct Want {
    pub memory: Option<u64>,
    pub cpu: Option<u32>,
    /// Whether a second copy on a machine that already holds one is
    /// worth having at all.
    ///
    /// **False for a stateless service and true for a database**, and
    /// the difference is what the copies are *for*. Two web containers
    /// on one node is more capacity and the plan's own measured case;
    /// two Postgres copies on one node is a standby of a primary on the
    /// same disk, which protects against nothing the primary was not
    /// already exposed to — and reads as redundancy on every page.
    ///
    /// **Found on the nodes**, 2026-08-25: draining the node holding the
    /// only remote standby moved it beside its own primary, and said so
    /// in the note without anything treating it as a problem.
    pub spread_required: bool,
}

/// Where it goes, or why nowhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Chosen {
        node: Option<String>,
        /// One line, for `replica.placed_note`.
        why: String,
    },
    /// **Never a quiet fall back to "here".** That is what
    /// `ensure_count(.., None)` does today, and it is how a node ends up
    /// over-committed with nothing having said so. The sentence carries
    /// the numbers, because a refusal without them is a form people
    /// fight by trying smaller values.
    Nowhere(String),
}

/// Pick a machine for one more copy.
///
/// Two passes. The first excludes every node already holding a copy of
/// this service, which is the spread; if that leaves nothing, the second
/// runs without it and the note says so. **Both are needed**: without
/// the exclusion a scheduler stacks three copies on the emptiest
/// machine, and without the fallback three copies on a two-node network
/// is unplaceable.
pub fn choose(candidates: &[Candidate], want: Want) -> Choice {
    let room = |candidate: &&Candidate| {
        candidate
            .rooms
            .admits(want.memory, want.cpu, 1, &candidate.name)
            .is_none()
    };
    let usable: Vec<&Candidate> = candidates
        .iter()
        .filter(|candidate| !candidate.evicted_here && !candidate.draining)
        .filter(room)
        .collect();

    let spread: Vec<&&Candidate> = usable
        .iter()
        .filter(|candidate| candidate.of_this_service == 0)
        .collect();

    let (pool, doubled_up): (Vec<&Candidate>, bool) = match spread.is_empty() {
        false => (spread.into_iter().copied().collect(), false),
        // **Placed anyway, and said out loud.** Doubling up is a real
        // loss for a service whose copies exist to be on different
        // machines — a standby beside its own primary protects against
        // nothing the primary was not already exposed to.
        //
        // It is still worth having, and that is Jorge's call rather than
        // a default: the copy keeps its own CPU allocation and its own
        // engine process, so it goes on serving reads. Refusing would
        // trade a real capability for a redundancy there is no second
        // machine to provide.
        //
        // So what was wrong was never the placement. It was that this
        // happened in silence — measured on the nodes, 2026-08-25, when
        // draining the only other node moved the only remote standby
        // home and nothing anywhere treated it as worth reading. The
        // note says it now, and `console::attention` asks about it.
        true => (usable.clone(), true),
    };

    let Some(best) = pool.into_iter().min_by_key(|candidate| rank(candidate)) else {
        return Choice::Nowhere(nowhere(candidates, want));
    };

    let free = crate::infra::node::memory::human(headroom(best));
    let why = match (doubled_up, candidates.len()) {
        // Named differently when the spread was the point, because the
        // two sentences mean different things to whoever reads the row:
        // one is "it is a bit fuller here", the other is "you asked for
        // two machines and there is one".
        (true, _) if want.spread_required => format!(
            "{} already runs a copy and no other node had room — these copies are meant to be \
             on different machines, so this one is capacity rather than redundancy ({free} free)",
            best.name
        ),
        (true, _) => format!(
            "{} already runs a copy; no other node had room ({free} free)",
            best.name
        ),
        (false, 1) => format!("{} was the only node with room ({free} free)", best.name),
        (false, count) => format!(
            "{} had the most room of {count} candidates ({free} free)",
            best.name
        ),
    };
    Choice::Chosen {
        node: best.node.clone(),
        why,
    }
}

/// The order, most preferred first. Every term is a `min`, so smaller
/// wins — headroom is negated for that reason.
///
/// **The spread is not in here**, and it was: `of_this_service` led this
/// tuple as well as filtering the pool above, which is one rule written
/// twice. Either alone did the job, so neither could be shown to matter
/// — a test disabling one passed against the other, which is exactly how
/// a rule quietly stops being enforced. It lives in the pass, because
/// that is also what decides which sentence the row gets.
///
/// The last term is not a preference and is not optional: **without a
/// total order two passes over the same rows can disagree**, and the
/// reconciliation that runs at every boot would move a copy each time it
/// ran. An arbitrary tie-break that never changes is worth more here
/// than a clever one that sometimes does.
fn rank(candidate: &Candidate) -> (std::cmp::Reverse<u64>, u32, u32, String) {
    (
        std::cmp::Reverse(headroom(candidate)),
        candidate.replicas,
        candidate.unbounded,
        // `None` is this node, and it sorts first. Deterministic, which
        // is the whole requirement; "here" losing every coin toss would
        // be a bias nobody asked for, and winning them is at least the
        // machine whose meters are live rather than reported.
        candidate.node.clone().unwrap_or_default(),
    )
}

/// What that machine would have left, in bytes.
///
/// **Worst fit**: the emptiest machine wins. Best fit packs tighter and
/// buys density this platform is not trying to sell — what it costs is
/// that the fullest machine gets the next copy too, and with no live
/// migration there is no way back out of that.
fn headroom(candidate: &Candidate) -> u64 {
    candidate
        .rooms
        .memory
        .allocatable
        .saturating_sub(candidate.rooms.memory.committed)
}

/// Why nowhere, naming the machines rather than the count.
fn nowhere(candidates: &[Candidate], want: Want) -> String {
    if candidates.is_empty() {
        return "no node has agreed to run this, so there is nowhere to put a copy".into();
    }
    let mut reasons: Vec<String> = candidates
        .iter()
        .map(|candidate| {
            if candidate.draining {
                return format!("{} is being taken out of service", candidate.name);
            }
            if candidate.evicted_here {
                return format!("{} threw a copy of this off", candidate.name);
            }
            candidate
                .rooms
                .admits(want.memory, want.cpu, 1, &candidate.name)
                .unwrap_or_else(|| format!("{} was not offered", candidate.name))
        })
        .collect();
    // Three, and then a count. A refusal nobody reads to the end is a
    // refusal that did not say anything.
    let more = reasons.len().saturating_sub(3);
    reasons.truncate(3);
    let listed = reasons.join("; ");
    match more {
        0 => format!("nowhere to put a copy: {listed}"),
        more => format!("nowhere to put a copy: {listed}; and {more} more"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::deploy::Room;

    const MB: u64 = 1024 * 1024;

    fn candidate(name: &str, free: u64) -> Candidate {
        Candidate {
            node: Some(format!("nd-{name}")),
            name: name.into(),
            rooms: Rooms {
                memory: Room {
                    allocatable: free,
                    known: true,
                    committed: 0,
                    already: 0,
                },
                cpu: Room {
                    allocatable: 4_000,
                    known: true,
                    committed: 0,
                    already: 0,
                },
            },
            of_this_service: 0,
            replicas: 0,
            unbounded: 0,
            evicted_here: false,
            draining: false,
        }
    }

    fn want(memory: u64) -> Want {
        Want {
            memory: Some(memory),
            cpu: None,
            spread_required: false,
        }
    }

    fn chosen(choice: Choice) -> Option<String> {
        match choice {
            Choice::Chosen { node, .. } => node,
            Choice::Nowhere(reason) => panic!("nowhere: {reason}"),
        }
    }

    /// The emptiest machine, and the sentence says which and why.
    #[test]
    fn the_copy_goes_where_there_is_most_room() {
        let pool = [
            candidate("alpine", 256 * MB),
            candidate("ubuntu", 900 * MB),
            candidate("pi", 512 * MB),
        ];
        let choice = choose(&pool, want(128 * MB));
        let Choice::Chosen { node, why } = choice else {
            panic!("nowhere");
        };
        assert_eq!(node.as_deref(), Some("nd-ubuntu"));
        assert!(why.contains("most room of 3 candidates"), "{why}");
        assert!(why.contains("900 MB free"), "{why}");
    }

    /// Copies of one service go on distinct machines first.
    ///
    /// The spread is what a second copy is *for*: two on the emptiest
    /// node is not redundancy, it is one machine's worth of failure with
    /// twice the memory spent on it.
    #[test]
    fn copies_of_one_service_are_spread_before_they_are_packed() {
        let mut pool = [candidate("ubuntu", 900 * MB), candidate("alpine", 256 * MB)];
        pool[0].of_this_service = 1;

        assert_eq!(
            chosen(choose(&pool, want(128 * MB))).as_deref(),
            Some("nd-alpine"),
            "the emptiest machine already had one"
        );
    }

    /// And the spread is a preference, not a wall.
    ///
    /// Three copies on a two-node network has to be placeable, or the
    /// count control is a lie. The exclusion is a first pass; when it
    /// leaves nothing the second runs without it, and the row says so —
    /// which is the difference between a decision and a surprise.
    #[test]
    fn a_third_copy_on_two_nodes_is_placed_and_says_so() {
        let mut pool = [candidate("ubuntu", 900 * MB), candidate("alpine", 256 * MB)];
        pool[0].of_this_service = 1;
        pool[1].of_this_service = 1;

        let Choice::Chosen { node, why } = choose(&pool, want(128 * MB)) else {
            panic!("a third copy on two nodes has to be placeable");
        };
        assert_eq!(node.as_deref(), Some("nd-ubuntu"), "the emptier of the two");
        assert!(why.contains("already runs a copy"), "{why}");
        assert!(why.contains("no other node had room"), "{why}");
    }

    /// Nothing fits, and the refusal names the machines and the numbers.
    ///
    /// **Never a quiet fall back to here.** A scheduler that placed
    /// somewhere rather than saying nowhere is how a node ends up
    /// over-committed with nothing having said so — and "refused" with
    /// no figures is a form somebody fights by trying smaller values.
    #[test]
    fn nowhere_is_an_answer_and_it_carries_the_numbers() {
        let pool = [candidate("alpine", 256 * MB), candidate("pi", 128 * MB)];
        let Choice::Nowhere(reason) = choose(&pool, want(512 * MB)) else {
            panic!("512 MB does not fit in 256");
        };
        assert!(reason.starts_with("nowhere to put a copy"), "{reason}");
        assert!(reason.contains("alpine has 256 MB to promise"), "{reason}");
        assert!(reason.contains("512 MB × 1 does not fit"), "{reason}");
    }

    /// A machine somebody is emptying is not given more to hold.
    ///
    /// The plainest rule on the list and the one a drain depends on:
    /// placing onto a node being drained means the drain has to move it
    /// again, and a drain that keeps finding new work never finishes.
    #[test]
    fn a_draining_node_is_not_given_more_to_hold() {
        let mut pool = [candidate("ubuntu", 900 * MB), candidate("alpine", 256 * MB)];
        pool[0].draining = true;

        assert_eq!(
            chosen(choose(&pool, want(128 * MB))).as_deref(),
            Some("nd-alpine"),
            "the emptiest machine is on its way out"
        );

        // And when it is the only one, the refusal says which it is —
        // "no room" would send somebody to look at the memory of a
        // machine that has plenty.
        pool[1].draining = true;
        let Choice::Nowhere(reason) = choose(&pool, want(128 * MB)) else {
            panic!("both are being emptied");
        };
        assert!(reason.contains("being taken out of service"), "{reason}");
    }

    /// A node that threw a copy off is not offered one again.
    ///
    /// Somebody on that machine did that deliberately — it is the one
    /// thing the operator of a node running somebody else's work can
    /// always do. Putting a copy straight back is asking them again, and
    /// a scheduler that argues with the person holding the machine is
    /// one they turn off.
    #[test]
    fn a_node_that_evicted_a_copy_is_not_offered_another() {
        let mut pool = [candidate("ubuntu", 900 * MB), candidate("alpine", 256 * MB)];
        pool[0].evicted_here = true;

        assert_eq!(
            chosen(choose(&pool, want(128 * MB))).as_deref(),
            Some("nd-alpine"),
            "the emptiest machine had thrown one off"
        );

        pool[1].evicted_here = true;
        let Choice::Nowhere(reason) = choose(&pool, want(128 * MB)) else {
            panic!("both had thrown one off");
        };
        assert!(reason.contains("threw a copy of this off"), "{reason}");
    }

    /// Two passes over the same rows pick the same machine.
    ///
    /// Not a nicety: reconciliation runs this at every boot, so a
    /// tie-break that depends on iteration order is a copy that moves
    /// every time the node restarts. Identical machines are the case
    /// that finds it, because every earlier term is equal.
    #[test]
    fn identical_machines_do_not_make_the_answer_wobble() {
        let pool = [
            candidate("bravo", 512 * MB),
            candidate("alpha", 512 * MB),
            candidate("charlie", 512 * MB),
        ];
        let first = chosen(choose(&pool, want(128 * MB)));
        for _ in 0..8 {
            assert_eq!(chosen(choose(&pool, want(128 * MB))), first);
        }
        assert_eq!(first.as_deref(), Some("nd-alpha"), "the lowest id, always");

        // And reversing the input does not reverse the answer.
        let mut backwards = pool.clone();
        backwards.reverse();
        assert_eq!(chosen(choose(&backwards, want(128 * MB))), first);
    }

    /// This node is a candidate like any other, spelled the way the row
    /// spells it.
    #[test]
    fn here_is_a_candidate_and_comes_out_as_none() {
        let mut here = candidate("this-one", 900 * MB);
        here.node = None;
        let pool = [here, candidate("alpine", 256 * MB)];

        assert_eq!(
            chosen(choose(&pool, want(128 * MB))),
            None,
            "`None` is what `replica.node_id` calls here"
        );
    }

    /// Nothing to choose from is its own sentence.
    ///
    /// The ordinary way to reach it is a network where nobody has
    /// granted `host`, which reads as a broken scheduler unless it says
    /// what is actually wrong.
    #[test]
    fn no_candidates_at_all_says_what_is_missing() {
        let Choice::Nowhere(reason) = choose(&[], want(128 * MB)) else {
            panic!("there is nowhere");
        };
        assert!(
            reason.contains("no node has agreed to run this"),
            "{reason}"
        );
    }
}
