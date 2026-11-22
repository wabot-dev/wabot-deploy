/// A passkey kept. When it was the account's first way to trust a
/// device, the browser it was made on is trusted, and its token is here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasskeyAddedResponse {
    pub device: Option<String>,
}
