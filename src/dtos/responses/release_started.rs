use serde::{Deserialize, Serialize};

/// An install is under way.
///
/// It carries the tag and nothing about how it went, because **the
/// browser that asked is about to lose the server it asked**: the
/// install ends by replacing this process. What happened is on the run
/// row, which is the thing that survives it.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReleaseStarted {
    pub tag: String,
}
