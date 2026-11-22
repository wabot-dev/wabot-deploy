use crate::domain::values::browser::Browser;

#[derive(Debug, Clone, Default)]
pub struct RedeemApprovalRequest {
    pub pending: Option<String>,
    pub code: String,
    pub trust: bool,
    pub browser: Browser,
}
