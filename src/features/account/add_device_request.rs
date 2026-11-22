#[derive(Debug, Clone, Default)]
pub struct AddDeviceRequest {
    /// Only a trusted device may hand out a link that trusts another.
    pub trusted: bool,
}
