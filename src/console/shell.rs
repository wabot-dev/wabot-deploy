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

/// Which theme to paint in.
///
/// Stored on the account, so the attribute is in the first byte of
/// HTML the browser sees. A theme applied by script runs after the page
/// paints, and the flash of the wrong one is worse than not offering
/// the choice at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    /// Follow the operating system. The default, and the only answer
    /// that keeps up when somebody's machine switches at sunset.
    System,
    Light,
    Dark,
}

impl Theme {
    pub fn parse(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// What to put in `data-theme`. Empty for `System`: no attribute
    /// means the media query decides, which is the whole point.
    fn attribute(self) -> &'static str {
        match self {
            Self::System => "",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// The one this button hands over to.
    ///
    /// Three states behind one control, so pressing it has to be a
    /// cycle. System first because it is the default and the answer
    /// that keeps up when a machine switches at sunset — somebody who
    /// has overridden it should reach "let the machine decide" again
    /// without hunting for it.
    fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    /// The icon for the state it is *in*, not the one it goes to.
    ///
    /// A control that pictured its destination would be read as a
    /// status by everybody who did not already know it was a button —
    /// the sun would mean "this is light" to most people looking at a
    /// dark page.
    fn icon(self) -> &'static str {
        match self {
            Self::System => MONITOR,
            Self::Light => SUN,
            Self::Dark => MOON,
        }
    }

    /// Said out loud, because an icon on its own is not.
    fn title(self) -> String {
        format!(
            "Theme: {}. Switch to {}.",
            self.label(),
            self.next().label()
        )
    }
}

/// XSS SAFETY: constants in this file, never a value from a request.
/// Raw because `rsx!` validates element names and does not know SVG's.
const MONITOR: &str = r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>"#;

const SUN: &str = r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M6.3 17.7l-1.4 1.4M19.1 4.9l-1.4 1.4"/></svg>"#;

const MOON: &str = r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/></svg>"#;

const GEAR: &str = r#"<svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9v0a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/></svg>"#;

/// Which top-level area a page belongs to. Decides what the side nav
/// shows and which top link is lit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Projects,
    /// Everything about the machine rather than about the work on it:
    /// the nodes, the people, the updates. One word in the header
    /// instead of three, because they are one question — "how is this
    /// installation set up" — and none of them is somewhere anybody
    /// spends the day.
    Settings,
}

/// Everything the frame needs to draw itself.
///
/// Owned, not borrowed. The frame names the current project in the
/// nav while the page draws that same project, and two `rsx!` closures
/// borrowing one value is a fight the compiler wins — for a handful of
/// short strings, copying is cheaper than the lifetime.
pub struct Frame {
    pub username: String,
    /// Whether to offer the node's own pages at all.
    pub admin: bool,
    pub area: Area,
    /// Every project, as (slug, name), for the selector.
    pub projects: Vec<(String, String)>,
    /// The slug of the one this page is about, if any.
    pub current: Option<String>,
    /// The request's path, for deciding which link is current.
    pub path: String,
    /// Whether the current project's own links are worth offering.
    /// The nav is where somebody looks for what they can do, so
    /// offering an action they will be refused is the nav lying.
    pub deploy: bool,
    pub theme: Theme,
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
            admin: account.is_admin(),
            area,
            projects: projects
                .iter()
                .map(|project| (project.slug.clone(), project.name.clone()))
                .collect(),
            current: current.map(|project| project.slug.clone()),
            path: path.into(),
            deploy: false,
            // Off the account, which every view already loaded to
            // decide what it may show. No cookie, no request-scoped
            // channel from the middleware, and the choice follows the
            // person to whatever machine they open this on.
            theme: account.theme,
        }
    }

    /// What the person may do in the project this page is about.
    pub fn allowing(mut self, access: crate::accounts::roles::Access) -> Self {
        self.deploy = access.may_deploy();
        self
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
            <div class="app-shell" data-theme=(self.theme.attribute())>
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
                    // The node, its people and what it runs on belong to
                    // whoever runs the node, and they are one place now
                    // rather than three top-level words. A member has
                    // projects and nothing else to see here.
                    @if self.admin {
                        <a href="/settings" class=(current(self.area == Area::Settings))
                           title="Settings">
                            (hypertext::Raw::dangerously_create(GEAR))
                            <span>("Settings")</span>
                        </a>
                    }
                </nav>
                <div class="topbar-right">
                    // One control, cycling. Three side by side spent the
                    // width of a menu on a preference somebody sets once
                    // — and the state it is in is still visible, because
                    // the icon shows *that* rather than where it goes.
                    <form method="post" action="/theme">
                        <input type="hidden" name="from" value=(&self.path)>
                        <button class="btn btn-ghost btn-icon" type="submit"
                                name="theme" value=(self.theme.next().as_str())
                                title=(self.theme.title())>
                            (hypertext::Raw::dangerously_create(self.theme.icon()))
                        </button>
                    </form>
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
                // The column and the nav inside it are two elements
                // because they do different jobs while the page
                // scrolls: the column holds the full-height background,
                // the inner box follows along. See `.side-inner`.
                <div class="side-inner">
                    @if self.area == Area::Projects {
                        (self.selector())
                        @if let Some(slug) = &self.current {
                            (self.project_nav(slug))
                        }
                    }
                    @if self.area == Area::Settings {
                        <p class="side-label">("Settings")</p>
                        <nav>
                            <a href="/nodes"
                               class=(current(self.path.starts_with("/nodes")))>("Nodes")</a>
                            <a href="/people"
                               class=(current(self.path == "/people"))>("People")</a>
                            <a href="/updates"
                               class=(current(self.path == "/updates"))>("Updates")</a>
                        </nav>
                    }
                </div>
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
                        // Same trap as everywhere else `selected` is
                        // written: it is boolean by presence, so with
                        // more than one project the browser took the
                        // last and this named the wrong one.
                        @for (slug, name) in &self.projects {
                            @if self.current.as_deref() == Some(slug.as_str()) {
                                <option value=(slug) selected>(name)</option>
                            } @else {
                                <option value=(slug)>(name)</option>
                            }
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

    /// The project's own pages.
    ///
    /// It grew when the overview was split: people and settings used to
    /// be cards below the services, found by scrolling past the thing
    /// everybody actually came for. A nav is where somebody looks for a
    /// page, which is the argument for putting them here rather than
    /// behind a button on the overview.
    fn project_nav(&self, slug: &str) -> impl Renderable + '_ {
        let base = format!("/projects/{slug}");
        let settings = format!("{base}/settings");

        rsx! {
            <nav>
                <a href=(&base) class=(current(self.path == base))>("Overview")</a>
                // Two items, and both are places. "Create service" was
                // neither — it was a button wearing a nav item's
                // clothes, sitting in the list of pages you can be *on*
                // while never being one you stay on. It belongs beside
                // the services it adds to, which is the overview.
                //
                // People moved into settings: who is in a project is a
                // setting of the project, and it was a page holding one
                // table.
                @if self.deploy {
                    <a href=(&settings)
                       class=(current(self.path == settings))>("Settings")</a>
                }
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
/* Block flow rather than a grid of two rows. A sticky element cannot
   leave its containing block, and a grid item's block is its grid
   area — as row one of this shell, the topbar had 3.25rem of room to
   move in and scrolled away with the page. */
.app-shell { min-height: 100vh; }
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
  position: sticky;
  top: 0;
  z-index: 3;
  /* The row template used to set this. Block flow does not, and a
     topbar that sizes to its content is a topbar that changes height
     between pages. */
  height: 3.25rem;
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
}
/* The column stays stretched so its background runs the height of the
   page; the nav inside it is what sticks. Sticking the column itself
   needs `align-self: start`, which collapses it to the height of the
   links and takes the background with it. */
.side-inner {
  position: sticky;
  top: calc(3.25rem + var(--sp-5));
  display: flex;
  flex-direction: column;
  gap: var(--sp-5);
  /* A project list longer than the viewport scrolls in place rather
     than pushing the links below the fold, where sticky cannot help. */
  max-height: calc(100vh - 3.25rem - calc(var(--sp-5) * 2));
  overflow-y: auto;
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
  /* Block flow again, and for the same reason as `.app-shell`: stacked
     in a grid the strip's area is its own row, which leaves it nothing
     to slide against. */
  .app-body { display: block; min-height: 0; }
  .sidebar {
    position: sticky;
    top: 3.25rem;
    z-index: 2;
    padding: var(--sp-3) var(--sp-4);
    overflow-x: auto;
  }
  .side-inner {
    position: static;
    flex-direction: row;
    align-items: end;
    gap: var(--sp-4);
    max-height: none;
    overflow-y: visible;
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

    /// `.side-inner` is what stays put while the page scrolls — the
    /// column around it only carries the background. A wrapper with no
    /// content of its own reads as removable, and removing it costs
    /// nothing visible until somebody scrolls.
    #[test]
    fn the_side_nav_sits_in_the_box_that_sticks() {
        let account = Account {
            theme: Theme::System,
            id: "a".into(),
            username: "someone".into(),
            role: crate::accounts::roles::NodeRole::Admin,
        };
        let frame = Frame::new(&account, Area::Settings, &[], None, "/people");
        let html = frame.render(String::new()).render().into_inner();

        let (_, after) = html
            .split_once(r#"class="side-inner""#)
            .expect("the sidebar wraps its nav in the sticky box");
        let (inside, _) = after
            .split_once("</aside>")
            .expect("and the box closes with the column");
        assert!(
            inside.contains(r#"href="/people""#),
            "the links are inside it: {inside}"
        );
    }
}
