use crate::domain::values::browser::Browser;

/// A password, for an account that still has one.
#[derive(Debug, Clone, Default)]
pub struct SignInRequest {
    pub username: String,
    pub password: String,
    pub scope: Option<String>,
    pub browser: Browser,
}
