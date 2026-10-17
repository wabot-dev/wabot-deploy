//! The form that makes a managed database.

use crate::domain::entities::database::Engine;
use crate::domain::policies::presets;
use crate::features::databases::texts::t;
use crate::shell::layouts::layout;
use hypertext::prelude::*;

use crate::features::databases::create_database_command::DEFAULT_PRESET;

pub struct NewDatabase<'a> {
    pub action: &'a str,
    pub back: &'a str,
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a NewDatabase<'a>) -> impl Renderable + 'a {
    let action = page.action;
    let back = page.back;
    let smallest = Engine::OFFERED
        .into_iter()
        .map(Engine::floor)
        .min()
        .unwrap_or(presets::SMALLEST);
    rsx! {
    (layout::style_tag())
        <div class="stack-sm">
            <h1>(t("Create database"))</h1>
        </div>
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        (wabot::ui::hypertext::island_bare("fields", rsx! {
        <form method="post" action=(action) class="card stack">
            <label for="name">(t("Name"))</label>
            <input id="name" name="name" type="text" autocomplete="off" required autofocus>

            <label for="engine">(t("Engine"))</label>
            <select id="engine" name="engine">
                @for engine in Engine::OFFERED {
                    @if engine == Engine::Postgres {
                        <option value=(engine.as_str()) selected>(engine.label())</option>
                    } @else {
                        <option value=(engine.as_str())>(engine.label())</option>
                    }
                }
            </select>

            @for engine in Engine::OFFERED {
                <div class="stack-sm" data-when=(format!("engine={}", engine.as_str()))>
                    <label for=(format!("{}-version", engine.as_str()))>(t("Version"))</label>
                    <select id=(format!("{}-version", engine.as_str()))
                            name=(format!("{}_version", engine.as_str()))>
                        @for version in engine.versions() {
                            @if *version == engine.default_version() {
                                <option value=(version) selected>(engine.label())" "(version)</option>
                            } @else {
                                <option value=(version)>(engine.label())" "(version)</option>
                            }
                        }
                    </select>
                </div>
            }
            <p class="field-hint">(t("The image is pulled from Docker Hub. The major version \
                 is fixed once the database exists: changing it is a data migration, not \
                 an image change."))</p>

            <label for="memory">(t("Memory"))</label>
            <select id="memory" name="memory">
                @for rung in presets::LADDER.into_iter().filter(|rung| *rung >= smallest) {
                    @if rung == DEFAULT_PRESET {
                        <option value=(rung.to_string()) selected>(presets::label(rung))</option>
                    } @else {
                        <option value=(rung.to_string())>(presets::label(rung))</option>
                    }
                }
            </select>
            <div class="stack-sm" data-when="engine=postgres">
                <p class="field-hint">(t("A ceiling on the container and the engine's own \
                     settings, together. Postgres is given a quarter of it for shared buffers \
                     and told to expect half of it as cache — its defaults alone would be \
                     killed on the smaller sizes."))</p>
                (crate::features::databases::database_cards_component::sizes_table())
            </div>
            <div class="stack-sm" data-when="engine=mysql">
                <p class="field-hint">(t("A ceiling on the container and the engine's own \
                     settings, together. MySQL's own defaults were killed at 256 MB, and \
                     below that it does not start however it is tuned — so 256 MB is the \
                     smallest size it takes, and its connections are counted against the \
                     ceiling."))</p>
                (crate::features::databases::database_cards_component::mysql_sizes_table())
            </div>

            <div class="actions">
                <button type="submit">(t("Create database"))</button>
                <a class="btn btn-ghost" href=(back)>(t("Cancel"))</a>
            </div>
        </form>
        }))
    }
}
