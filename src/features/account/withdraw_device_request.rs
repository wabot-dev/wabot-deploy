#[derive(Debug, Clone, Default)]
pub struct WithdrawDeviceRequest {
    pub device_id: String,
    pub address: Option<String>,
}
