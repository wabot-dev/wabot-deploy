/// Which passkey the refused device used, because somebody else has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRefusedResponse {
    pub passkey_id: String,
}
