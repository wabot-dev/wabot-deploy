/// The six digits the new device has to type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceApprovedResponse {
    pub code: String,
    pub name: String,
    pub expires_at: i64,
}
