use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Send `hostname` to `to`, which has to be one of the service's own.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct AddServiceRedirectRequest {
    #[min_length(1)]
    #[max_length(253)]
    pub hostname: String,
    #[min_length(1)]
    #[max_length(253)]
    pub to: String,
}
