use serde::{Deserialize, Serialize};

/// Where the node's own name ended up.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum DomainSettledResponse {
    /// The name was checked and saved. The caller has a sentence to show
    /// that the page's own stream cannot: that it resolved *before*
    /// anything was asked of an authority.
    Named(String),
    Cleared,
}
