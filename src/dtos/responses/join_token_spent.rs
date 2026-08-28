use serde::{Deserialize, Serialize};

/// What a join token did — which is not always joining.
///
/// **The terms are read before they are agreed to**, so spending one
/// has two outcomes and the first of them writes nothing.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum Spent {
    /// The terms were read but nothing agreed to yet. The token comes
    /// back so the console can show what it asks for **before** it is
    /// spent, which is the whole point of the order.
    Review(String),
    /// Named, because the page says who this node now answers to.
    Joined(String),
}
