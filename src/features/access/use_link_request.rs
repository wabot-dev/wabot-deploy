use crate::domain::values::browser::{Browser, Ceremony};

#[derive(Debug, Clone, Default)]
pub struct UseLinkRequest {
    pub token: String,
    pub scope: Option<String>,
    pub passkey: Ceremony,
    pub trust: bool,
    pub browser: Browser,
}
