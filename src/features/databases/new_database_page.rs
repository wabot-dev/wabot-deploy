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
    /// What this node is called, for the first choice.
    pub here: Option<&'a str>,
    /// The other nodes that agreed to keep data for this one.
    pub elsewhere: &'a [crate::infra::network::Node],
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

            <label class="check">
                <input type="checkbox" name="custom_image" value="1">
                (t("Use my own image"))
            </label>
            <div class="stack-sm" data-when="custom_image">
                <p class="form-warning">(t("A custom image is not guaranteed to work. The node \
                     starts it exactly as it starts its own, so it has to be built on the \
                     engine's official image, at the version chosen above, with the same \
                     entrypoint and the same user. Anything else may refuse to start, or start \
                     and fail later, and keeping it up to date is up to whoever publishes it."))</p>
                <label for="image">(t("Image reference"))</label>
                <input id="image" name="image" type="text" autocomplete="off" class="mono"
                       placeholder="docker.io/pgvector/pgvector:pg17-trixie"
                       data-required-when="custom_image">
                <p class="field-hint">(t("With its registry written out. For vector search, "))
                    <code>"docker.io/pgvector/pgvector:pg17-trixie"</code>
                    (t(" is PostgreSQL 17 with pgvector. The node checks the version the image \
                     says it runs before starting it, and the image stays fixed once the \
                     database exists."))</p>
            </div>

            @for engine in Engine::OFFERED {
                <div class="stack-sm" data-when=(format!("engine={}", engine.as_str()))>
                    <label for=(format!("{}-memory", engine.as_str()))>(t("Memory"))</label>
                    <select id=(format!("{}-memory", engine.as_str()))
                            name=(format!("{}_memory", engine.as_str()))>
                        @for rung in presets::LADDER.into_iter().filter(|rung| *rung >= engine.floor()) {
                            @if rung == DEFAULT_PRESET {
                                <option value=(rung.to_string()) selected>(presets::label(rung))</option>
                            } @else {
                                <option value=(rung.to_string())>(presets::label(rung))</option>
                            }
                        }
                    </select>
                    @match engine {
                        Engine::Mysql => {
                            <p class="field-hint">(t("A ceiling on the container and the engine's own \
                                 settings, together. MySQL's own defaults were killed at 256 MB, and \
                                 below that it does not start however it is tuned — so 256 MB is the \
                                 smallest size it takes, and its connections are counted against the \
                                 ceiling."))</p>
                            (crate::features::databases::database_cards_component::mysql_sizes_table())
                        }
                        _ => {
                            <p class="field-hint">(t("A ceiling on the container and the engine's own \
                                 settings, together. Postgres is given a quarter of it for shared buffers \
                                 and told to expect half of it as cache — its defaults alone would be \
                                 killed on the smaller sizes. 128 MB is the smallest size it takes: in \
                                 64 MB the kernel can kill initdb before it finishes writing its \
                                 catalogs."))</p>
                            (crate::features::databases::database_cards_component::sizes_table())
                        }
                    }
                </div>
            }

            @if !page.elsewhere.is_empty() {
                <label for="on">(t("Runs on"))</label>
                <select id="on" name="on">
                    <option value="" selected>(page.here.unwrap_or(t("this node")))</option>
                    @for node in page.elsewhere {
                        <option value=(&node.id)>(&node.name)</option>
                    }
                </select>
                <p class="field-hint">(t("Where it runs, for good: a database is not moved once it \
                     exists. Services reach it by its name only from that node."))</p>
            }

            <div class="actions">
                <button type="submit">(t("Create database"))</button>
                <a class="btn btn-ghost" href=(back)>(t("Cancel"))</a>
            </div>
        </form>
        }))
    }
}
