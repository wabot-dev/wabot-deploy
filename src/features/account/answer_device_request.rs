/// "It was me" or "It was not me", about the request a new device opened.
#[derive(Debug, Clone, Default)]
pub struct AnswerDeviceRequest {
    pub request_id: String,
    /// Whether the session answering is on a device the account trusts:
    /// only one of those may let another in.
    pub trusted: bool,
    pub address: Option<String>,
}
