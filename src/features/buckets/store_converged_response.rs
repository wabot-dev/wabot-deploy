/// What a change to a store's rows left in Garage.
///
/// The rows are saved either way; this says whether the running store holds
/// them yet, so the page reports what Garage answered rather than what the
/// row says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreConvergedResponse {
    Taken,
    NotRunning,
    Refused(String),
}

impl StoreConvergedResponse {
    pub fn of(outcome: Result<bool, String>) -> Self {
        match outcome {
            Ok(true) => StoreConvergedResponse::Taken,
            Ok(false) => StoreConvergedResponse::NotRunning,
            Err(reason) => StoreConvergedResponse::Refused(reason),
        }
    }
}
