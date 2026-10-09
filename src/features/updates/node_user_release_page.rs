use hypertext::prelude::*;

use super::node_user_updates_page::{day_of, install_form, rendered_notes, state_line};
use crate::infra::update::github::{Release, Version};
use crate::web::views::layouts::layout;
use crate::web::views::texts::language::t;

pub struct ReleaseView<'a> {
    pub release: &'a Release,
    pub current: Option<Version>,
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a ReleaseView<'a>) -> impl Renderable + 'a {
    let release = page.release;
    let current = page.current;
    rsx! {
        (layout::style_tag())
        <p class="crumb"><a href="/updates">(t("Updates"))</a></p>
        <h1>(&release.name)</h1>
        <p class="tagline">
            (&release.tag)
            @if !release.published_at.is_empty() {
                (t(" · published "))(day_of(&release.published_at))
            }
        </p>

        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }

        <section class="card stack">
            (state_line(release, current))
            @if release.installable() && current != Some(release.version) {
                (install_form(release, "btn", "Install this release"))
            }
        </section>

        <section class="card stack">
            <p class="card-label">(t("What is in it"))</p>
            (rendered_notes(&release.notes))
            @if !release.html_url.is_empty() {
                <p class="tile-detail">
                    <a href=(&release.html_url) rel="noreferrer noopener" target="_blank">
                        (t("Read it on GitHub"))
                    </a>
                </p>
            }
        </section>
    }
}
