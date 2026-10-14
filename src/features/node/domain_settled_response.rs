use serde::{Deserialize, Serialize};

/// Where the node's own name ended up.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum DomainSettledResponse {
    /// The name was checked and saved. The caller has a sentence to show
    /// that the page's own stream cannot: that it resolved *before*
    /// anything was asked of an authority.
    /// With the addresses the name resolved to: the node cannot tell
    /// whether they are its own, so it says what it found rather than
    /// "resolves here" — which it said of a name pointing elsewhere.
    Named {
        domain: String,
        addresses: Vec<String>,
    },
    Cleared,
}
