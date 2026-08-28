use serde::{Deserialize, Serialize};

/// What the database answers to now, and what it answered to before.
///
/// `was` carries the old name because the caller has to withdraw it —
/// a rename that left the previous name routed would keep serving a
/// certificate for a name nobody owns any more.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DatabaseNamed {
    /// What it answered to before, when the rename replaced a name.
    pub was: Option<String>,
    pub now: String,
}
