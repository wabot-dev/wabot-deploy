use serde::{Deserialize, Serialize};

/// A link into an organisation.
///
/// Carries the organisation back because the address the link hangs off
/// is that organisation's own — the node's would sign them up and bounce
/// them on their first click.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PersonInvitedIntoOrganisationResponse {
    pub organisation: String,
    pub token: String,
}
