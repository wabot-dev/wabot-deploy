use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Text where numbers are meant, so the refusal can say which one it
/// could not read rather than serde's *invalid digit*.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct AddRecordRequest {
    #[min_length(1)]
    pub project: String,
    #[min_length(1)]
    pub zone: String,
    #[max_length(253)]
    #[serde(default)]
    pub name: String,
    #[min_length(1)]
    pub kind: String,
    #[min_length(1)]
    #[max_length(4096)]
    pub value: String,
    #[max_length(5)]
    #[serde(default)]
    pub priority: String,
    #[max_length(6)]
    #[serde(default)]
    pub ttl: String,
}
