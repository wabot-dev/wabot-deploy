use serde::{Deserialize, Serialize};
use wabot::prelude::*;

#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct CreateZoneRequest {
    #[min_length(1)]
    #[max_length(253)]
    pub name: String,
}
