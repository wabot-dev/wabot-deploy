use crate::domain::values::browser::{Browser, Ceremony};

#[derive(Debug, Clone, Default)]
pub struct PasskeySignInRequest {
    pub passkey: Ceremony,
    pub scope: Option<String>,
    pub browser: Browser,
}
