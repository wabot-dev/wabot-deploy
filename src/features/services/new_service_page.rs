//! The form that makes a service.

use crate::features::services::texts::t;
use crate::shell::layouts::layout;
use hypertext::prelude::*;

pub struct NewService<'a> {
    pub action: &'a str,
    pub back: &'a str,
    pub query_error: Option<&'a str>,
    /// What this node is called, for the first choice.
    pub here: Option<&'a str>,
    /// The other nodes that agreed to run containers for this one.
    pub elsewhere: &'a [crate::infra::network::Node],
}

pub fn render<'a>(page: &'a NewService<'a>) -> impl Renderable + 'a {
    let action = page.action;
    let back = page.back;
    rsx! {
    (layout::style_tag())
        <div class="stack-sm">
            <h1>(t("Create service"))</h1>
        </div>
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        <form method="post" action=(action) class="card stack">
            <label for="name">(t("Name"))</label>
            <input id="name" name="name" type="text" autocomplete="off" required autofocus>

            <label for="image">(t("Image, if you have one"))</label>
            // Not `required`. A service for your own application
            // has no image until you have pushed one, and this
            // field being mandatory meant typing a reference for
            // something that did not exist yet. Reported by
            // Jorge.
            <input id="image" name="image" type="text" autocomplete="off" class="mono"
                   placeholder="docker.io/library/nginx:alpine">
            <p class="field-hint">(t("A reference containerd can resolve. Fully qualified — \
                 there is no implicit registry here."))</p>
            <p class="field-hint">(t("Leave it empty for your own application: the service \
                 waits, and the first push to its own repository becomes its first \
                 release. The page will show you the two commands."))</p>

            @if !page.elsewhere.is_empty() {
                <label for="on">(t("Runs on"))</label>
                <select id="on" name="on">
                    <option value="" selected>(page.here.unwrap_or(t("this node")))</option>
                    @for node in page.elsewhere {
                        <option value=(&node.id)>(&node.name)</option>
                    }
                </select>
                <p class="field-hint">(t("Where its first copy runs. Copies can be moved, or added on other \
                     nodes, from the service's settings."))</p>
            }

            <label for="env">(t("Environment"))</label>
            <textarea id="env" name="env" autocomplete="off" rows="6" class="mono"
                      placeholder="KEY=value"></textarea>
            <p class="field-hint">(t("One KEY=value per line. Everything after the first = is \
                 the value, so a value may contain one."))</p>

            <div class="actions">
                <button type="submit">(t("Create service"))</button>
                <a class="btn btn-ghost" href=(back)>(t("Cancel"))</a>
            </div>
        </form>
    }
}
