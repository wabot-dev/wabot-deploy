#[derive(Debug, Clone, Default)]
pub struct RemovePasskeyRequest {
    pub passkey_id: String,
    pub address: Option<String>,
}
