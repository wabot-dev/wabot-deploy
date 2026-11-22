use crate::domain::values::browser::{Browser, Ceremony};

/// Spend an invitation and become somebody here: a name, and the passkey
/// the browser made for it, when it could make one.
#[derive(Debug, Clone, Default)]
pub struct JoinRequest {
    pub token: String,
    pub username: String,
    pub passkey: Ceremony,
    pub browser: Browser,
}
