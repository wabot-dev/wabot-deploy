use serde::{Deserialize, Serialize};
use wabot::prelude::*;

/// When the node backs itself up, where it sends it and how long it
/// keeps it.
///
/// **The numbers arrive as text, on purpose.** Typing them here would
/// hand a wrong one to serde, which can say *invalid digit* and nothing
/// else; the use case says *"7 days" is not a number of days* and names
/// the field it read. Two of them — the hour and the weekday — do not
/// refuse at all: an unreadable value takes the default and is clamped,
/// because a backup that runs at the wrong hour is better than a form
/// that will not save.
///
/// `archiving` is a `bool` and not text because a checkbox has no value
/// when it is off: what the browser sends is the field's **absence**,
/// so the controller is the only place that can tell "unticked" from
/// "not on this form".
#[derive(Debug, Clone, Default, Deserialize, Serialize, Validate)]
pub struct SetBackupPlanRequest {
    /// Empty means no destination, which is a plan that measures and
    /// keeps nothing rather than a missing field.
    #[serde(default)]
    pub destination: String,
    #[serde(default)]
    pub cadence: String,
    #[serde(default)]
    pub hour: String,
    #[serde(default)]
    pub weekday: String,
    #[serde(default)]
    pub keep_days: String,
    #[serde(default)]
    pub archiving: bool,
}
