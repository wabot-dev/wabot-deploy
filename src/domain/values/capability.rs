use serde::{Deserialize, Serialize};

/// A thing one node can be asked to do.
///
/// Serialised **lowercase, which is what `name()` already spells** — a
/// request DTO carries these, and a second spelling for one thing is how
/// a value written by one side stops being read by the other.
/// `a_capability_serialises_as_the_name_it_is_known_by` holds them
/// together, over `ALL`, so a new variant cannot be added to only one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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

    #[test]
    fn a_capability_serialises_as_the_name_it_is_known_by() {
        for capability in Capability::ALL {
            let written = serde_json::to_string(&capability).expect("serialise");
            assert_eq!(written, format!("{:?}", capability.name()));
            let read: Capability = serde_json::from_str(&written).expect("read back");
            assert_eq!(read, capability);
        }
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
}
