use serde::{Deserialize, Serialize};

/// What draining a node moved, or that it was put back into service.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub enum NodeDrainedResponse {
    BackInService,
    /// Draining is **convergent, not atomic**: a node that is not
    /// answering keeps its containers until it collects the errand with
    /// the slot gone, so the count and the refusals are both worth
    /// showing.
    Moved {
        moved: usize,
        refused: Vec<String>,
    },
}
