//! The form that makes a managed database.

use crate::policies::postgres;
use crate::policies::presets;
use crate::views::layouts::layout;
use crate::views::texts::language::t;
use hypertext::prelude::*;

use crate::commands::create_database::DEFAULT_PRESET;

pub struct NewDatabase<'a> {
    pub action: &'a str,
    pub back: &'a str,
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a NewDatabase<'a>) -> impl Renderable + 'a {
    let action = page.action;
    let back = page.back;
    rsx! {
    (layout::style_tag())
        <div class="stack-sm">
            <h1>(t("Create database"))</h1>
        </div>
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        <form method="post" action=(action) class="card stack">
            <label for="name">(t("Name"))</label>
            <input id="name" name="name" type="text" autocomplete="off" required autofocus>

            <label for="version">(t("Version"))</label>
            <select id="version" name="version">
                @for version in postgres::VERSIONS {
                    @if version == postgres::DEFAULT_VERSION {
                        <option value=(version) selected>("PostgreSQL ")(version)</option>
                    } @else {
                        <option value=(version)>("PostgreSQL ")(version)</option>
                    }
                }
            </select>
            <p class="field-hint">(t("The image is pulled from Docker Hub. The major version \
                 is fixed once the database exists: changing it is a data migration, not \
                 an image change."))</p>

            <label for="memory">(t("Memory"))</label>
            <select id="memory" name="memory">
                @for rung in presets::LADDER {
                    @if rung == DEFAULT_PRESET {
                        <option value=(rung.to_string()) selected>(presets::label(rung))</option>
                    } @else {
                        <option value=(rung.to_string())>(presets::label(rung))</option>
                    }
                }
            </select>
            <p class="field-hint">(t("A ceiling on the container and the engine's own \
                 settings, together. Postgres is given a quarter of it for shared buffers \
                 and told to expect half of it as cache — its defaults alone would be \
                 killed on the smaller sizes."))</p>
            (crate::views::components::database_cards::sizes_table())

            <div class="actions">
                <button type="submit">(t("Create database"))</button>
                <a class="btn btn-ghost" href=(back)>(t("Cancel"))</a>
            </div>
        </form>
    }
}
