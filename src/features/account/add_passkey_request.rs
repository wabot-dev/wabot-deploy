use crate::domain::values::browser::{Browser, Ceremony};

#[derive(Debug, Clone, Default)]
pub struct AddPasskeyRequest {
    pub passkey: Ceremony,
    pub browser: Browser,
    /// The session asking, which the device it is on is bound to when this
    /// passkey is the one that trusts it.
    pub session: String,
}
