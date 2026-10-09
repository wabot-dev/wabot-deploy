use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Spend an invitation and become somebody here.
///
/// **The password's length is not checked here**, on purpose: the rule
/// belongs to whatever makes an account, it is the one refusal the
/// invitee can act on, and a copy of it in this file is a second place
/// it can drift. What this covers is that a field arrived at all.
#[derive(Debug, Clone, Deserialize, Serialize, Validate)]
pub struct JoinRequest {
    #[min_length(1)]
    pub token: String,
    #[min_length(1)]
    pub username: String,
    #[min_length(1)]
    pub password: String,
}
