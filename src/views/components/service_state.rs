//! What a service is doing, and the buttons that change it.
//!
//! Status is a coloured dot plus a word — never a colour alone, which
//! says nothing to somebody who cannot tell them apart.

use hypertext::prelude::*;

use crate::console::projects::StateCell;

///
/// Rendered from the same [`StateCell`] the stream sends, so the first
/// paint and every update after it come from one place. They used to be
/// two functions that had to be kept in step, which is the kind of pair
/// that drifts.
pub fn state_badge(cell: &StateCell) -> impl Renderable + '_ {
    rsx! {
        <span class=(cell.badge)>
            <span class=(cell.dot)></span>(crate::views::texts::language::word(&cell.word))
        </span>
    }
}

/// One control, disabled or not.
///
/// Written twice rather than `disabled=(busy)`, for the same reason:
/// `disabled="false"` disables. There is no way to spell "no attribute"
/// in an `rsx!` attribute value, so the branch is on the element.
/// The play and the stop, for whichever page is showing a service.
///
/// One definition, because the project's list and the service's own
/// page must not disagree about which control applies — and they had
/// no way not to, since the detail page simply had neither. A service
/// you can see the state of and cannot start is a page that shows a
/// problem and withholds the fix.
///
/// Both actions are rendered and one is hidden, so the stream can swap
/// them by toggling: an island may hide what does not apply, not build
/// what does. The check that counts is on the POST; this is so the page
/// does not offer what it will refuse.
pub fn deploy_controls<'a>(
    project_slug: &'a str,
    service_slug: &'a str,
    cell: &'a StateCell,
    // Where to go afterwards. The form carries it because only the form
    // knows which page it is on — the same two controls live on the
    // list and on the service's own page.
    from: &'a str,
) -> impl Renderable + 'a {
    rsx! {
        <form method="post" data-action="deploy"
              class=(crate::views::components::action_button::shown(cell.action == "deploy"))
              action=(format!("/projects/{project_slug}/services/{service_slug}/deploy"))>
            <input type="hidden" name="from" value=(from)>
            (crate::views::components::action_button::action_button("Deploy", crate::views::components::action_button::PLAY, cell.busy))
        </form>
        <form method="post" data-action="stop"
              class=(crate::views::components::action_button::shown(cell.action == "stop"))
              action=(format!("/projects/{project_slug}/services/{service_slug}/stop"))>
            <input type="hidden" name="from" value=(from)>
            (crate::views::components::action_button::action_button("Stop", crate::views::components::action_button::STOP, cell.busy))
        </form>
    }
}
