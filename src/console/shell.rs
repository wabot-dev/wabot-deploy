//! The frame every signed-in page sits in.
//!
//! Topbar, side nav, project selector — the same shape as wabot
//! console, so somebody who knows one knows this. What differs is
//! underneath: this renders on the server with no client router, so
//! "which link is current" is decided here from the request's path
//! rather than by a runtime watching history.
//!
//! ## The selector is a form, not a dropdown that navigates
//!
//! A `<select>` that routes on change needs JavaScript. A `<select>`
//! inside a form with a submit button works with none, and on a
//! console whose whole job is to still work when things are broken,
//! that is the right side of the trade.

use hypertext::prelude::*;

use crate::accounts::Account;
use crate::platform::projects::Project;

use super::assets;

/// Which top-level area a page belongs to. Decides what the side nav
/// shows and which top link is lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Projects,
    Nodes,
}

/// Everything the frame needs to draw itself.
///
/// Owned, not borrowed. The frame names the current project in the
/// nav while the page draws that same project, and two `rsx!` closures
/// borrowing one value is a fight the compiler wins — for a handful of
/// short strings, copying is cheaper than the lifetime.
pub struct Frame {
    pub username: String,
    pub area: Area,
    /// Every project, as (slug, name), for the selector.
    pub projects: Vec<(String, String)>,
    /// The slug of the one this page is about, if any.
    pub current: Option<String>,
    /// The request's path, for deciding which link is current.
    pub path: String,
}

impl Frame {
    /// The frame for a page, from what every view already has.
    pub fn new(
        account: &Account,
        area: Area,
        projects: &[Project],
        current: Option<&Project>,
        path: impl Into<String>,
    ) -> Self {
        Self {
            username: account.username.clone(),
            area,
            projects: projects
                .iter()
                .map(|project| (project.slug.clone(), project.name.clone()))
                .collect(),
            current: current.map(|project| project.slug.clone()),
            path: path.into(),
        }
    }

    /// The whole frame, with the page's own markup inside it.
    ///
    /// Takes the body already rendered. Nesting the page's `rsx!`
    /// inside the frame's would have both of them borrowing the same
    /// project at once — one to name it in the nav, the other to draw
    /// it — and the closure `rsx!` builds captures by move. Rendering
    /// first ends the page's borrow before the frame takes its own.
    pub fn render(&self, inner: String) -> impl Renderable + '_ {
        rsx! {
            <div class="app-shell">
                (self.topbar())
                <div class="app-body">
                    (self.sidebar())
                    <main>(hypertext::Raw::dangerously_create(&inner))</main>
                </div>
            </div>
        }
    }

    fn topbar(&self) -> impl Renderable + '_ {
        rsx! {
            <header class="topbar">
                <a class="brand" href="/">
                    <img
                        src=(format!("{}/wabot-logo.png", assets::MOUNT))
                        alt="Wabot" width="20" height="20">
                    <span>("wabot-deploy")</span>
                </a>
                <nav>
                    <a href="/" class=(current(self.area == Area::Projects))>("Projects")</a>
                    <a href="/nodes" class=(current(self.area == Area::Nodes))>("Nodes")</a>
                </nav>
                <div class="topbar-right">
                    <span class="who">(&self.username)</span>
                    // A form, not a link: signing out changes state,
                    // and a GET that changes state is one a prefetcher
                    // can fire.
                    <form method="post" action="/sign-out">
                        <button class="btn btn-ghost btn-sm" type="submit">("Sign out")</button>
                    </form>
                </div>
            </header>
        }
    }

    fn sidebar(&self) -> impl Renderable + '_ {
        rsx! {
            <aside class="sidebar">
                @if self.area == Area::Projects {
                    (self.selector())
                    @if let Some(slug) = &self.current {
                        (self.project_nav(slug))
                    }
                }
                @if self.area == Area::Nodes {
                    <p class="side-label">("Node")</p>
                    <nav>
                        <a href="/nodes" class=(current(self.path == "/nodes"))>("All nodes")</a>
                    </nav>
                }
            </aside>
        }
    }

    /// The project switcher.
    fn selector(&self) -> impl Renderable + '_ {
        rsx! {
            <form method="post" action="/select-project" class="workspace-field">
                <label class="side-label" for="project">("Project")</label>
                <div class="workspace">
                    <select id="project" name="project">
                        @for (slug, name) in &self.projects {
                            <option
                                value=(slug)
                                selected=(self.current.as_deref() == Some(slug.as_str()))>
                                (name)
                            </option>
                        }
                    </select>
                    <button class="btn btn-secondary btn-sm" type="submit">("Go")</button>
                </div>
                @if self.projects.is_empty() {
                    <p class="side-hint">("No projects yet.")</p>
                }
            </form>
        }
    }

    fn project_nav(&self, slug: &str) -> impl Renderable + '_ {
        let base = format!("/projects/{slug}");
        let services = format!("{base}/services/new");

        rsx! {
            <nav>
                <a href=(&base) class=(current(self.path == base))>("Overview")</a>
                <a href=(&services) class=(current(self.path == services))>("Create service")</a>
            </nav>
        }
    }
}

/// `class="active"`, or nothing.
///
/// An empty string rather than an absent attribute: `rsx!` wants a
/// value, and `class=""` renders as nothing a stylesheet can match.
fn current(active: bool) -> &'static str {
    if active {
        "active"
    } else {
        ""
    }
}

/// The shell's own layout. Colour, type and radii come from the design
/// system's tokens — this is arrangement, which is the console's
/// business. No borders, no shadows: separation is background
/// contrast.
pub const CSS: &str = r#"
.app-shell {
  display: grid;
  grid-template-rows: 3.25rem 1fr;
  min-height: 100vh;
}
.app-body {
  display: grid;
  grid-template-columns: 15rem minmax(0, 1fr);
  min-height: calc(100vh - 3.25rem);
}
/* A page title is a label, not a headline. The design system's `h1`
   is `--fs-4xl` — a marketing size — and next to 15px body text it
   reads as a banner. wabot console uses `--fs-3xl` for the same job,
   but its titles are words like "Services" while these are hostnames,
   so this goes one step further down and lets a long one break rather
   than push the page sideways. */
.app-body > main h1 {
  font-size: var(--fs-2xl);
  margin: 0;
  overflow-wrap: anywhere;
}
.app-body > main h2 { font-size: var(--fs-xl); margin: 0; }

.app-body > main {
  padding: var(--sp-8) var(--sp-8) var(--sp-12);
  max-width: 66rem;
  width: 100%;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--sp-6);
}

.topbar {
  display: flex;
  align-items: center;
  gap: var(--sp-5);
  padding: 0 var(--sp-5);
  background: rgb(var(--c-bg-sunken));
}
.topbar .brand {
  display: inline-flex;
  align-items: center;
  gap: var(--sp-2);
  font-weight: 600;
  letter-spacing: -0.02em;
  color: rgb(var(--c-fg));
  text-decoration: none;
}
.topbar nav { display: flex; gap: var(--sp-1); }
.topbar nav a {
  padding: 0.35rem var(--sp-3);
  border-radius: var(--r-md);
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-muted));
  text-decoration: none;
}
.topbar nav a.active {
  background: rgb(var(--c-bg-raised));
  color: rgb(var(--c-fg));
  font-weight: 500;
}
.topbar-right {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}
.who { color: rgb(var(--c-fg-muted)); font-size: var(--fs-sm); }

.sidebar {
  background: rgb(var(--c-bg-sunken));
  padding: var(--sp-5) var(--sp-3);
  display: flex;
  flex-direction: column;
  gap: var(--sp-5);
}
.side-label {
  font: 500 var(--fs-xs)/1 var(--font-sans);
  letter-spacing: var(--tracking-caps);
  text-transform: uppercase;
  color: rgb(var(--c-fg-muted));
  padding: 0 var(--sp-3);
  margin: 0 0 var(--sp-2);
}
.side-hint {
  padding: 0 var(--sp-3);
  margin: var(--sp-2) 0 0;
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-faint));
}
.sidebar nav { display: flex; flex-direction: column; gap: 2px; }
.sidebar nav a {
  padding: 0.45rem var(--sp-3);
  border-radius: var(--r-md);
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-body));
  text-decoration: none;
}
.sidebar nav a.active {
  background: rgb(var(--c-bg-raised));
  color: rgb(var(--c-fg));
  font-weight: 500;
}

.workspace { display: flex; gap: var(--sp-2); align-items: center; }
.workspace select {
  background: rgb(var(--c-bg-contrast));
  font-size: var(--fs-sm);
  padding: 0.4rem var(--sp-3);
}

@media (max-width: 60rem) {
  .app-body { grid-template-columns: minmax(0, 1fr); }
  .sidebar {
    flex-direction: row;
    align-items: end;
    gap: var(--sp-4);
    padding: var(--sp-3) var(--sp-4);
    overflow-x: auto;
  }
  .sidebar nav { flex-direction: row; }
  .app-body > main { padding: var(--sp-6) var(--sp-4) var(--sp-10); }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_current_link_is_marked() {
        assert_eq!(current(true), "active");
        assert_eq!(current(false), "");
    }
}
