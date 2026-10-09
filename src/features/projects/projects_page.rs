//! Every project this account can see.

use crate::web::views::layouts::layout;
use crate::web::views::texts::language::t;
use hypertext::prelude::*;

pub struct Projects<'a> {
    pub projects: &'a [crate::domain::entities::project::Project],
    pub owns: bool,
    pub concerns: &'a [crate::web::console::attention::Concern],
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a Projects<'a>) -> impl Renderable + 'a {
    let projects = page.projects;
    let owns = page.owns;
    let concerns = page.concerns;
    rsx! {
            (layout::style_tag())
            <div class="split">
                <h1>(t("Projects"))</h1>
                // Hidden rather than disabled, and the POST
                // refuses either way: a button that cannot work
                // is a question somebody has to answer twice.
                @if owns {
                    <a class="btn" href="/projects/new">(t("Create project"))</a>
                }
            </div>
            @if let Some(message) = &page.query_error {
                (layout::error_note(message))
            }

            // Above the list, and only when there is something.
            // A panel that is always there is part of the
            // wallpaper by the second week.
            (crate::web::views::components::attention_card::card(concerns))

            @if projects.is_empty() {
                // No second "Create project" here: the one in the
                // header is on this page whether or not the list is
                // empty, and two buttons a hand's width apart that
                // do the same thing read as two different things.
                <section class="empty">
                    @if owns {
                        <p>(t("No projects yet."))</p>
                    } @else {
                        <p>(t("This node owns nothing, by its own setting. It runs what other \
                             nodes ask it to run; anything that arrives that way is \
                             administered from the node that sent it."))</p>
                    }
                </section>
            } @else {
                <div class="grid">
                    @for project in projects {
                        <a class="card tile" href=(format!("/projects/{}", project.slug))>
                            <p class="tile-name">(&project.name)</p>
                            <p class="tile-detail">(&project.slug)</p>
                        </a>
                    }
                </div>
            }
    }
}
