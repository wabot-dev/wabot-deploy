/// A session started, and what its cookies carry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedInResponse {
    pub session: String,
    /// Made on a trusted device, so it outlives the browser.
    pub lasting: bool,
    /// A device token to set, when this sign-in trusted the browser.
    pub device: Option<String>,
}
