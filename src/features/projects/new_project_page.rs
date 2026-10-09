//! The form that makes a project.

use crate::views::layouts::layout;
use crate::views::texts::language::t;
use hypertext::prelude::*;

pub struct NewProject<'a> {
    pub query_error: Option<&'a str>,
}

pub fn render<'a>(page: &'a NewProject<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <h1>(t("Create project"))</h1>
        @if let Some(message) = &page.query_error {
            (layout::error_note(message))
        }
        <form method="post" action="/projects" class="card stack">
            <label for="name">(t("Name"))</label>
            <input id="name" name="name" type="text" autocomplete="off" required autofocus>
            <p class="field-hint">(t("The slug is derived from the name, and it is what \
                 hostnames and containerd labels are built from."))</p>

            <div class="actions">
                <button type="submit">(t("Create project"))</button>
                <a class="btn btn-ghost" href="/">(t("Cancel"))</a>
            </div>
        </form>
    }
}
