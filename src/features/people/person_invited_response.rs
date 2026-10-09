use serde::{Deserialize, Serialize};

/// A link was minted. Only the token travels: the address it hangs off
/// is the console's to build, because which name a person should open is
/// a question about this deployment rather than about the invitation.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PersonInvitedResponse {
    pub token: String,
}
