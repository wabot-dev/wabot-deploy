use serde::{Deserialize, Serialize};

/// The plan was stored.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BackupPlanSavedResponse {
    /// Whether the timestamped-log setting moved. The caller redeploys
    /// the managed engines when it did, because it is something they
    /// read at startup.
    pub archiving_changed: bool,
}
