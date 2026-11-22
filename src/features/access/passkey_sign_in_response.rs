use crate::features::access::signed_in_response::SignedInResponse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasskeySignInResponse {
    SignedIn(SignedInResponse),
    /// A device nobody trusts: the token of the request it opened, for
    /// the browser to hold until a trusted device approves it.
    Awaiting(String),
}
