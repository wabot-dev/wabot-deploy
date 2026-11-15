use hypertext::prelude::*;

use crate::shell::layouts::layout;
use crate::shell::texts::language::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Project,
    Service,
    Node,
    Organisation,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Self::Project => t("Project"),
            Self::Service => t("Service"),
            Self::Node => t("Node"),
            Self::Organisation => t("Organisation"),
        }
    }
}

/// One thing a search can land on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub kind: Kind,
    pub label: String,
    /// The other string it answers to: a slug, or a node's id.
    pub key: String,
    pub href: String,
    /// What it is inside, when two things can share a name: a service's
    /// project.
    pub inside: Option<String>,
}

impl Hit {
    pub fn new(kind: Kind, label: &str, key: &str, href: impl Into<String>) -> Self {
        Self {
            kind,
            label: label.to_string(),
            key: key.to_string(),
            href: href.into(),
            inside: None,
        }
    }

    pub fn inside(mut self, of: &str) -> Self {
        self.inside = Some(of.to_string());
        self
    }
}

pub struct Results<'a> {
    pub asked: &'a str,
    pub hits: &'a [Hit],
}

pub fn render<'a>(page: &'a Results<'a>) -> impl Renderable + 'a {
    rsx! {
        (layout::style_tag())
        <h1>(t("Go to"))</h1>
        <form method="get" action="/go" class="card stack">
            <label for="q">(t("Name"))</label>
            <div class="row">
                <input id="q" name="q" type="search" autocomplete="off" value=(page.asked)>
                <button type="submit">(t("Go"))</button>
            </div>
        </form>
        @if page.asked.is_empty() {
            <section class="empty">
                <p>(t("Type the name of something here to go to it."))</p>
            </section>
        } @else if page.hits.is_empty() {
            <section class="empty">
                <p>(t("Nothing here is called that."))</p>
            </section>
        } @else {
            <section class="card">
                <table>
                    <tbody>
                        @for hit in page.hits {
                            <tr>
                                <td><a href=(&hit.href)>(&hit.label)</a></td>
                                <td class="tile-detail">
                                    (hit.kind.label())
                                    @if let Some(inside) = &hit.inside {
                                        (" · ")(inside)
                                    }
                                </td>
                            </tr>
                        }
                    </tbody>
                </table>
            </section>
        }
    }
}
