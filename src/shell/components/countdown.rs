//! How long something that expires has left: the minutes as the page was
//! rendered, and the `countdown` island counting them down to the second.

use hypertext::prelude::*;

use crate::shell::texts::language::t;
use crate::shell::texts::sentences;

pub fn countdown(seconds_left: i64) -> impl Renderable {
    let props = serde_json::json!({
        "left": seconds_left.max(0),
        "running": t("Expires in {}"),
        "over": t("Expired"),
    });
    let text = sentences::expires_in(seconds_left);
    let inner = rsx! {
        <span class="countdown" data-countdown>(text)</span>
    }
    .render()
    .into_inner();
    wabot::ui::hypertext::island(
        "countdown",
        &props,
        hypertext::Raw::dangerously_create(inner),
    )
}
