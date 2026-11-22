#[derive(Debug, Clone, Default)]
pub struct RenamePasskeyRequest {
    pub passkey_id: String,
    /// Empty gives it back the name it was born with.
    pub label: String,
}
