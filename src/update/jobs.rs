//! Installing a release as a job, not as a spawn.
//!
//! ## Why a job
//!
//! Because the process is about to be replaced, and that is the whole
//! point of the work. `deploy::jobs` explains the same choice for the
//! same reason: a spawned task loses what it was doing when the binary
//! is swapped, and the runner's row in SQLite does not. Here it matters
//! twice over — the thing being spawned is the swap.
//!
//! ## Why an errand does not call `update::apply` directly
//!
//! An errand is settled by the node that carried it out, over the
//! network, *after* `carry_out` returns. `apply` ends by restarting the
//! service, so calling it inline would take the process with it before
//! the settlement was ever sent — and the node that asked would see an
//! instruction that never came back. Queueing puts the settlement first
//! and the install second, without either one having to know about the
//! other.
//!
//! Even so, **the guard is the version and not the order.** If this
//! process dies before the errand is settled, the errand is collected
//! again — and by then the node either is on that release, in which case
//! it is satisfied and settles as such, or is not, in which case trying
//! again is exactly right. Nothing here depends on winning a race.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Put this release on this node, and restart into it.
#[command("install-release")]
#[derive(Debug, Serialize, Deserialize, Validate)]
pub struct InstallRelease {
    /// As GitHub names it. Resolved again here rather than carried as a
    /// URL: the release list is one answer per machine, and this job may
    /// run a moment or a minute after the errand that queued it.
    #[is_not_empty]
    pub tag: String,
    /// Who asked, when it was not somebody at this node's own console.
    ///
    /// Logged and not stored: `update_run` records an account, and a node
    /// is not one. What the asking node needs is on its own errand row,
    /// and what this node's operator needs is a line in the journal
    /// saying the install was not theirs.
    pub asked_by: Option<String>,
}

/// No retries, deliberately.
///
/// A release that will not install is not a transient failure: the
/// download refused, the checksum did not match, or the binary will not
/// run — and none of those improve by being tried again four times. What
/// they do improve by is being *said*, which the run row and the report's
/// note both do. The node that asked can ask again, which is one event
/// and one new errand.
// `#[injectable]` first and the handler attribute on the *struct*, both
// as `deploy::jobs` found out: the expansion parses an `ItemStruct`, and
// without the first there is no `inject` for `register_transients!`.
#[injectable]
#[command_handler(InstallRelease, retry_delays = [])]
pub struct InstallHandler {
    database: Arc<wabot::sqlite::SqliteDatabase>,
    config: Arc<crate::config::Config>,
}

impl InstallHandler {
    async fn handle(&self, data: InstallRelease) -> Result<(), AsyncError> {
        if let Some(node) = &data.asked_by {
            tracing::info!(node = %node, tag = %data.tag, "installing a release another node asked for");
        }
        // Nothing after a successful `apply` runs: it ends by restarting
        // the service. A failure is on the run row, and travels back to
        // whoever asked on the next report — see `collect::update_note`.
        crate::update::apply(
            self.database.clone(),
            (*self.config).clone(),
            crate::update::shared(),
            &data.tag,
            None,
        )
        .await
        .map_err(|error| AsyncError::Handler(error.to_string()))
    }
}
