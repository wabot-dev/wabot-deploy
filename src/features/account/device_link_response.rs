/// A link that works once, for five minutes, on the device that opens it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkResponse {
    pub token: String,
    pub expires_at: i64,
}
