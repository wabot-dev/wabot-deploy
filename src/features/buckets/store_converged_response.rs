use crate::infra::deploy::stores::Applied;

/// What a change to a store's rows left in Garage.
///
/// The rows are saved either way; this says whether the running store holds
/// them yet, so the page reports what Garage answered rather than what the
/// row says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreConvergedResponse {
    Taken,
    NotRunning,
    /// Sent to the node running the store, which applies it when it next
    /// collects its errands.
    Sent(String),
    Refused(String),
}

impl StoreConvergedResponse {
    pub fn of(outcome: Result<Applied, String>) -> Self {
        match outcome {
            Ok(Applied::Taken) => StoreConvergedResponse::Taken,
            Ok(Applied::NotRunning) => StoreConvergedResponse::NotRunning,
            Ok(Applied::Sent(node)) => StoreConvergedResponse::Sent(node),
            Err(reason) => StoreConvergedResponse::Refused(reason),
        }
    }
}
