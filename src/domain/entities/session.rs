use serde::{Deserialize, Serialize};

/// How long a proof of who somebody is counts for something that makes a
/// way in: adding a passkey, approving a device, issuing a link.
pub const FRESH_MS: i64 = 5 * 60_000;

/// A signed-in browser: which device it was made on, and when its holder
/// last proved who they are.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub token_hash: String,
    pub device_id: Option<String>,
    pub verified_at: Option<i64>,
}

impl Session {
    /// Made on a device its account trusts.
    pub fn trusted(&self) -> bool {
        self.device_id.is_some()
    }

    /// Proved who they are recently enough to make a way in.
    pub fn fresh(&self, now: i64) -> bool {
        self.verified_at.is_some_and(|at| now - at <= FRESH_MS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proof_counts_for_five_minutes() {
        let session = Session {
            token_hash: "h".into(),
            device_id: None,
            verified_at: Some(1_000),
        };
        assert!(session.fresh(1_000 + FRESH_MS));
        assert!(!session.fresh(1_001 + FRESH_MS));
        assert!(!Session {
            verified_at: None,
            ..session
        }
        .fresh(1_000));
    }
}
