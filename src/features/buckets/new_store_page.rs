//! The form that makes a store of buckets.

use hypertext::prelude::*;

use crate::domain::policies::{garage, presets};
use crate::features::buckets::create_store_command::DEFAULT_PRESET;
use crate::features::buckets::texts::t;
use crate::shell::layouts::layout;

pub struct NewStore<'a> {
    pub action: &'a str,
    pub back: &'a str,
    pub query_error: Option<&'a str>,
    /// What an empty name field becomes, with the store's own name left as a
    /// placeholder, so the page says it before anything is typed.
    pub derived: Option<&'a str>,
    pub enabled: bool,
    /// What this node is called, for the first choice of where it runs.
    pub here: Option<&'a str>,
    /// The other nodes that would keep it: those that agreed to keep data
    /// for this one and run a release that knows stores.
    pub elsewhere: &'a [crate::infra::network::Node],
}

pub fn render<'a>(page: &'a NewStore<'a>) -> impl Renderable + 'a {
    rsx! {
    (layout::style_tag())
        <div class="stack-sm">
            <h1>(t("Create store"))</h1>
        </div>
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        @if !page.enabled {
            <section class="card stack">
                <p class="tile-detail">(t("This node runs no stores of buckets. An administrator \
                     turns them on in the node's settings, under Capabilities."))</p>
            </section>
        } @else {
            <form method="post" action=(page.action) class="card stack">
                <label for="name">(t("Name"))</label>
                <input id="name" name="name" type="text" autocomplete="off" required autofocus>

                <label for="hostname">(t("Domain"))</label>
                @if let Some(derived) = page.derived {
                    <input id="hostname" name="hostname" type="text" autocomplete="off"
                           class="mono" placeholder=(derived)>
                    <p class="field-hint">(t("The store's endpoint, and what each of its buckets is a \
                         subdomain of: a bucket called uploads is reached at uploads.<domain>. Empty is \
                         the one shown, with the store's name in place of <name>. It has to point at \
                         this node, and so does every bucket's name — one record *.<domain> covers \
                         them all. It cannot be changed afterwards."))</p>
                } @else {
                    <input id="hostname" name="hostname" type="text" autocomplete="off"
                           class="mono" required>
                    <p class="field-hint">(t("The store's endpoint, and what each of its buckets is a \
                         subdomain of: a bucket called uploads is reached at uploads.<domain>. This node \
                         has no domain to derive one under, so it has to be typed. It has to point at \
                         this node, and so does every bucket's name — one record *.<domain> covers \
                         them all. It cannot be changed afterwards."))</p>
                }

                <label for="memory">(t("Memory"))</label>
                <select id="memory" name="memory">
                    @for rung in presets::LADDER.into_iter().filter(|rung| *rung >= garage::FLOOR) {
                        @if rung == DEFAULT_PRESET {
                            <option value=(rung.to_string()) selected>(presets::label(rung))</option>
                        } @else {
                            <option value=(rung.to_string())>(presets::label(rung))</option>
                        }
                    }
                </select>
                <p class="field-hint">(t("A ceiling on the container. An eighth of it holds what is \
                     waiting to reach the disk; the rest is the engine, its metadata and the requests \
                     in flight."))</p>

                <label for="disk">(t("Disk"))</label>
                <input id="disk" name="disk" type="text" autocomplete="off" required
                       placeholder="10 GB">
                <p class="field-hint">(t("What its buckets may hold together. Every bucket is given a \
                     maximum, and their sum has to fit here — a full bucket answers its client with an \
                     error rather than filling a disk other services share."))</p>

                @if !page.elsewhere.is_empty() {
                    <label for="on">(t("Runs on"))</label>
                    <select id="on" name="on">
                        <option value="" selected>(page.here.unwrap_or(t("this node")))</option>
                        @for node in page.elsewhere {
                            <option value=(&node.id)>(&node.name)</option>
                        }
                    </select>
                    <p class="field-hint">(t("Where Garage runs and its objects are kept, for good: a \
                         store is not moved once it exists. Its name is served from this node either \
                         way, and its backups are taken where it runs."))</p>
                }

                (crate::shell::components::allowlist_field::field("allowlist", "allowlist",
                    t("Who may connect"),
                    &crate::domain::values::allowlist::Allowlist::default()))

                <div class="actions">
                    <button type="submit">(t("Create store"))</button>
                    <a class="btn btn-ghost" href=(page.back)>(t("Cancel"))</a>
                </div>
            </form>
        }
    }
}
