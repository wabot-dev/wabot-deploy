use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// Hand an organisation to somebody.
///
/// **One field, and that is the point.** The node's operator used to
/// pick a role and a project here, which made them an administrator
/// inside a frontier that is not theirs — they could put somebody into
/// one organisation's project while inviting them into another's. What
/// the operator decides is *who answers for this organisation*;
/// everything after that is the organisation's own, and it has a console
/// to do it from.
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct InviteOwner {
    #[min_length(1)]
    pub organisation: String,
}
