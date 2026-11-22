/// A link that signs one person in once, within fifteen minutes, on
/// their own console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignInLinkResponse {
    pub token: String,
    pub tenant_id: Option<String>,
}
