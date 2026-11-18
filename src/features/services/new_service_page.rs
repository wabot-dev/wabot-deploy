//! The form that makes a service.

use crate::domain::policies::presets;
use crate::domain::values::allowlist::Allowlist;
use crate::features::services::texts::t;
use crate::shell::components::allowlist_field;
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
    pub left: Option<u64>,
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

            <label for="memory-size">(t("Memory"))</label>
            <input id="memory-size" name="memory" type="text" list="memory-sizes"
                   autocomplete="off" required
                   value=(presets::size_field(presets::DEFAULT_MEMORY))>
            <datalist id="memory-sizes">
                @for rung in presets::LADDER {
                    <option value=(presets::size_field(rung))></option>
                }
            </datalist>
            <p class="field-hint">(t("A size, like 512 MB or 1.5 GB — a bare \
                 number is megabytes. Over this the kernel kills the container \
                 rather than letting it swap, which is the outcome to want: a \
                 process quietly swapping is invisible until everything on the \
                 node is slow."))</p>
            @if let Some(left) = page.left {
                <p class="field-hint">
                    (crate::infra::node::memory::human(left))(t(" is what this node has \
                         left to promise. A service is refused memory it does not have."))
                </p>
            }

            <label for="cpu-size">(t("CPU"))</label>
            <input id="cpu-size" name="cpu" type="text" list="cpu-sizes"
                   autocomplete="off" required
                   value=(presets::cores_field(presets::DEFAULT_MILLICORES))>
            <datalist id="cpu-sizes">
                @for rung in presets::CPU_LADDER {
                    <option value=(presets::cores_field(rung))></option>
                }
            </datalist>
            <p class="field-hint">(t("Cores, like 0.5 or 2 — or millicores with an \
                 m, like 500m. Over this the container is throttled, not killed: it \
                 runs slowly rather than stopping. Unlike memory, a node may promise \
                 more CPU than it has: services share its cores, and together never \
                 take what it keeps for itself."))</p>

            <label for="env">(t("Environment"))</label>
            <textarea id="env" name="env" autocomplete="off" rows="6" class="mono"
                      placeholder="KEY=value"></textarea>
            <p class="field-hint">(t("One KEY=value per line. Everything after the first = is \
                 the value, so a value may contain one."))</p>

            // **Asked for here, and born with it.** Decided by Jorge: an
            // empty list admits nobody, so a service is open only to whom
            // somebody wrote down when making it.
            (allowlist_field::field("allowlist", "allowlist", t("Who may connect to its names and published ports"),
                &Allowlist::default()))

            <div class="actions">
                <button type="submit">(t("Create service"))</button>
                <a class="btn btn-ghost" href=(back)>(t("Cancel"))</a>
            </div>
        </form>
    }
}
