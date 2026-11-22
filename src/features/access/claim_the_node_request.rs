use crate::domain::values::browser::{Browser, Ceremony};

#[derive(Debug, Clone, Default)]
pub struct ClaimTheNodeRequest {
    pub setup_token: String,
    pub username: String,
    pub passkey: Ceremony,
    pub browser: Browser,
}
