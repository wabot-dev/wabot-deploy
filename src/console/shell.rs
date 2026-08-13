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
use super::language::t;

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

    /// The icon for the state it is *in*, not the one it goes to.
    ///
    /// A control that pictured its destination would be read as a
    /// status by everybody who did not already know it was a button —
    /// the sun would mean "this is light" to most people looking at a
    /// dark page.
    fn icon(self) -> &'static str {
        match self {
            // Unreachable through the control, and kept for the
            // account that has never pressed it. See `toggle`.
            Self::System => MONITOR,
            Self::Light => SUN,
            Self::Dark => MOON,
        }
    }

    /// Said out loud, because an icon on its own is not.
    fn title(self) -> String {
        let goes_to = match self {
            Self::Dark => Self::Light,
            _ => Self::Dark,
        };
        format!("Theme: {}. Switch to {}.", self.label(), goes_to.label())
    }
}

/// XSS SAFETY: constants in this file, never a value from a request.
/// Raw because `rsx!` validates element names and does not know SVG's.
const MONITOR: &str = r#"<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>"#;

const SUN: &str = r#"<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M6.3 17.7l-1.4 1.4M19.1 4.9l-1.4 1.4"/></svg>"#;

const MOON: &str = r#"<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M21 12.8A9 9 0 1 1 11.2 3a7 7 0 0 0 9.8 9.8z"/></svg>"#;

const ARROW_LEFT: &str = r#"<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M19 12H5"/><polyline points="12 19 5 12 12 5"/></svg>"#;

const SIGN_OUT: &str = r#"<svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M9 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h4"/><polyline points="16 17 21 12 16 7"/><line x1="21" x2="9" y1="12" y2="12"/></svg>"#;

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
    pub language: crate::console::language::Language,
}

/// The last segment of a path when it names an action rather than a
/// thing.
///
/// Translated, because these are the only words in a trail that are
/// prose — the rest are names somebody typed or ids a machine made.
fn verb(segment: &str) -> Option<String> {
    Some(match segment {
        "new" => t("New").to_string(),
        "settings" => t("Settings").to_string(),
        "join" => t("Join").to_string(),
        _ => return None,
    })
}

/// What to call a collection that is a page in its own right, when it
/// is one.
///
/// `None` covers two cases and they are the same case: a collection
/// nested under something — `/projects/x/services` — and `/projects`
/// itself, whose listing lives at `/`. Neither is a route, and a crumb
/// linking to one is a link to a 404. A test found this by walking every
/// shape rather than by anybody noticing.
fn collection(segment: &str) -> Option<String> {
    Some(match segment {
        "nodes" => t("Nodes").to_string(),
        "people" => t("People").to_string(),
        "updates" => t("Updates").to_string(),
        _ => return None,
    })
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
            language: account.language,
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
        // The frame's own words, in the account's language. The page
        // inside it is already a rendered string by the time it gets
        // here — each view scopes its own, because that is where its
        // strings are read. See `language::scoped`.
        let language = self.language;
        let top =
            crate::console::language::scoped(language, || self.topbar().render().into_inner());
        let side =
            crate::console::language::scoped(language, || self.sidebar().render().into_inner());
        let crumbs =
            crate::console::language::scoped(language, || self.crumbs().render().into_inner());
        rsx! {
            <div class="app-shell" data-theme=(self.theme.attribute())>
                (hypertext::Raw::dangerously_create(&top))
                <div class="app-body">
                    (hypertext::Raw::dangerously_create(&side))
                    <main>
                        (hypertext::Raw::dangerously_create(&crumbs))
                        (hypertext::Raw::dangerously_create(&inner))
                    </main>
                </div>
            </div>
        }
    }

    /// Where this page sits, and the way back out of it.
    ///
    /// Rendered by the frame rather than by each view, and **derived
    /// from the path** rather than passed in. Both of those are the same
    /// decision: a trail every view has to remember to supply is a trail
    /// some view does not have, and the frame already knows the path
    /// because it decides which nav link is current from it.
    ///
    /// The way back is the crumb before the last — **always**, because
    /// that is what a parent is. So the control is an arrow rather than
    /// a word: spelling its destination out put "Projects" on the bar
    /// twice, once in the button and once in the crumb it points at,
    /// which is not an edge case but every page one level deep. The name
    /// lives in the tooltip, where it costs no room and still answers
    /// "where does this go".
    ///
    /// An icon button rather than a bare glyph, for the hit area: a `←`
    /// sitting against a text link is a few pixels wide and next to
    /// something much easier to hit by accident.
    fn crumbs(&self) -> impl Renderable + '_ {
        let trail = self.trail();
        // Nothing to show on a root page: a single crumb naming the page
        // you are looking at is furniture.
        let show = trail.len() > 1;
        let parent = match trail.len() {
            0 | 1 => None,
            n => Some(trail[n - 2].clone()),
        };

        rsx! {
            @if show {
                <nav class="crumbs" aria-label=(t("Breadcrumb"))>
                    @if let Some((label, Some(href))) = &parent {
                        <a class="crumb-back btn btn-ghost btn-icon" href=(href)
                           title=(format!("{} {label}", t("Back to")))
                           aria-label=(format!("{} {label}", t("Back to")))>
                            (hypertext::Raw::dangerously_create(ARROW_LEFT))
                        </a>
                    }
                    <ol>
                        @for (index, (label, href)) in trail.iter().enumerate() {
                            <li>
                                // The last one is where you are. A link
                                // to the page you are on is a link that
                                // teaches somebody it does nothing.
                                @if index + 1 == trail.len() {
                                    <span aria-current="page">(label)</span>
                                } @else if let Some(href) = href {
                                    <a href=(href)>(label)</a>
                                } @else {
                                    <span>(label)</span>
                                }
                            </li>
                        }
                    </ol>
                </nav>
            }
        }
    }

    /// The trail, as `(label, href)` from the root to this page.
    ///
    /// The path is read in **`collection/id` pairs**, so `projects` and
    /// `services` are not crumbs of their own — `/projects/x/services`
    /// is not a page, and a crumb linking to it would be a link to a
    /// 404. What is left over at the end is a verb (`new`, `settings`),
    /// which is the current page and needs no link.
    fn trail(&self) -> Vec<(String, Option<String>)> {
        let segments: Vec<&str> = self.path.split('/').filter(|s| !s.is_empty()).collect();
        let mut trail = vec![match self.area {
            Area::Projects => (t("Projects").to_string(), Some("/".to_string())),
            Area::Settings => (t("Settings").to_string(), None),
        }];

        let mut index = 0;
        while index < segments.len() {
            let segment = segments[index];
            // A verb ends the trail: it is what this page *does*, and
            // there is nothing under it.
            if let Some(label) = verb(segment) {
                trail.push((label, None));
                return trail;
            }
            // A collection and the thing inside it. The href is the
            // whole prefix, which is the page that thing has.
            let id = match segments.get(index + 1) {
                Some(id) if verb(id).is_none() => id,
                // A collection with nothing usable under it. Some are
                // pages — `/nodes` — and the nested ones are not, so a
                // crumb appears only for the ones that answer.
                _ => {
                    if let Some(label) = collection(segment) {
                        trail.push((label, Some(format!("/{}", segments[..=index].join("/")))));
                    }
                    index += 1;
                    continue;
                }
            };
            trail.push((
                self.label_for(segment, id),
                Some(format!("/{}", segments[..=index + 1].join("/"))),
            ));
            index += 2;
        }
        trail
    }

    /// What to call one thing in the trail.
    ///
    /// A project is named rather than slugged when the frame knows the
    /// name — it holds every project for the selector — because the
    /// name is what somebody typed. Everything else is its slug or its
    /// id, unchanged: those are what the rows and the terminal call it,
    /// and prettifying an id is how a page stops matching `ctr`.
    fn label_for(&self, collection: &str, id: &str) -> String {
        match collection {
            "projects" => self
                .projects
                .iter()
                .find(|(slug, _)| slug == id)
                .map(|(_, name)| name.clone())
                .unwrap_or_else(|| id.to_string()),
            _ => id.to_string(),
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
                    <a href="/" class=(current(self.area == Area::Projects))>(t("Projects"))</a>
                    // The node, its people and what it runs on belong to
                    // whoever runs the node, and they are one place now
                    // rather than three top-level words. A member has
                    // projects and nothing else to see here.
                    @if self.admin {
                        <a href="/settings" class=(current(self.area == Area::Settings))
                           title=(t("Settings"))>
                            (hypertext::Raw::dangerously_create(GEAR))
                            <span>(t("Settings"))</span>
                        </a>
                    }
                </nav>
                <div class="topbar-right">
                    (self.languages())
                    (self.toggle())
                    // A form, not a link: signing out changes state,
                    // and a GET that changes state is one a prefetcher
                    // can fire.
                    //
                    // An icon, with the account's name in its tooltip.
                    // The name used to sit in the bar as a label of its
                    // own, which spent room on something nobody needs to
                    // read twice a day — and it is still the thing you
                    // want to check before pressing *this* button, so it
                    // moved into the one place you look at first.
                    <form method="post" action="/sign-out">
                        <button class="btn btn-ghost btn-icon" type="submit"
                                title=(format!("{} · {}", t("Sign out"), self.username))
                                aria-label=(t("Sign out"))>
                            (hypertext::Raw::dangerously_create(SIGN_OUT))
                        </button>
                    </form>
                </div>
            </header>
        }
    }

    /// English or Spanish, in the two letters somebody scans for.
    ///
    /// A word rather than a flag: a flag is a country and a language is
    /// not one — Spanish is not Spain's, and picking any flag for
    /// English starts an argument the console does not need to have.
    fn languages(&self) -> impl Renderable + '_ {
        rsx! {
            <form method="post" action="/language">
                <input type="hidden" name="from" value=(&self.path)>
                <button class="btn btn-ghost btn-icon" type="submit" name="language"
                        value=(self.language.other().as_str())
                        title=(self.language.offer())>
                    (self.language.other().short())
                </button>
            </form>
        }
    }

    /// Light or dark, and nothing else.
    ///
    /// `System` is still the stored default — an account that has never
    /// pressed this follows the machine, which is the answer that keeps
    /// up when it switches at sunset — but it is not one of the two
    /// things the control offers.
    ///
    /// Which leaves a problem worth the trouble: while an account is on
    /// `System`, the **server cannot know** whether that machine is
    /// light or dark, and the icon is supposed to show the state you
    /// are in. So both are rendered and the media query picks. CSS
    /// knows what the server does not, and this console has to work
    /// with scripting off anyway. After one press the answer is stored
    /// and only one button is drawn.
    fn toggle(&self) -> impl Renderable + '_ {
        rsx! {
            <form method="post" action="/theme">
                <input type="hidden" name="from" value=(&self.path)>
                @if self.theme == Theme::System {
                    <button class="btn btn-ghost btn-icon when-light" type="submit"
                            name="theme" value="dark" title="Switch to dark">
                        (hypertext::Raw::dangerously_create(SUN))
                    </button>
                    <button class="btn btn-ghost btn-icon when-dark" type="submit"
                            name="theme" value="light" title="Switch to light">
                        (hypertext::Raw::dangerously_create(MOON))
                    </button>
                } @else {
                    <button class="btn btn-ghost btn-icon" type="submit" name="theme"
                            value=(match self.theme {
                                Theme::Dark => "light",
                                _ => "dark",
                            })
                            title=(self.theme.title())>
                        (hypertext::Raw::dangerously_create(self.theme.icon()))
                    </button>
                }
            </form>
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
                        <p class="side-label">(t("Settings"))</p>
                        <nav>
                            <a href="/nodes"
                               class=(current(self.path.starts_with("/nodes")))>(t("Nodes"))</a>
                            <a href="/people"
                               class=(current(self.path == "/people"))>(t("People"))</a>
                            <a href="/updates"
                               class=(current(self.path == "/updates"))>(t("Updates"))</a>
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
                <label class="side-label" for="project">(t("Project"))</label>
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
                <a href=(&base) class=(current(self.path == base))>(t("Overview"))</a>
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
                       class=(current(self.path == settings))>(t("Settings"))</a>
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
            language: crate::console::language::Language::En,
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

    /// The shapes a trail takes, and the one it must never take.
    ///
    /// `/projects/x/services` is not a page, so `services` cannot be a
    /// crumb of its own — the path is read in `collection/id` pairs for
    /// exactly that reason, and a crumb linking to a 404 is worse than
    /// no crumb.
    #[test]
    fn a_trail_never_links_to_a_path_that_is_not_a_page() {
        let account = Account {
            theme: Theme::System,
            id: "a".into(),
            username: "someone".into(),
            role: crate::accounts::roles::NodeRole::Admin,
            language: crate::console::language::Language::En,
        };
        let project = Project {
            id: "prj-1".into(),
            name: "Database test".into(),
            slug: "db-test".into(),
            created_at: 0,
            origin_node_id: None,
        };

        let trail_for = |area, path: &str| {
            Frame::new(&account, area, std::slice::from_ref(&project), None, path).trail()
        };

        // A project is named rather than slugged: the name is what
        // somebody typed, and the frame already knows it.
        let project_page = trail_for(Area::Projects, "/projects/db-test");
        assert_eq!(
            project_page,
            vec![
                ("Projects".to_string(), Some("/".to_string())),
                (
                    "Database test".to_string(),
                    Some("/projects/db-test".to_string())
                ),
            ]
        );

        let service = trail_for(Area::Projects, "/projects/db-test/services/orders");
        assert_eq!(
            service.last().expect("a last crumb").0,
            "orders",
            "the page you are on comes last"
        );
        assert_eq!(service.len(), 3, "{service:?}");

        let settings = trail_for(Area::Projects, "/projects/db-test/services/orders/settings");
        assert_eq!(
            settings.last().expect("last"),
            &("Settings".to_string(), None)
        );

        // A verb has nothing under it, so it is where the trail stops.
        let new = trail_for(Area::Projects, "/projects/new");
        assert_eq!(new.last().expect("last"), &("New".to_string(), None));

        // A collection that *is* a page keeps its own crumb.
        let nodes = trail_for(Area::Settings, "/nodes");
        assert_eq!(
            nodes.last().expect("last"),
            &("Nodes".to_string(), Some("/nodes".to_string()))
        );
        let one_node = trail_for(Area::Settings, "/nodes/nd-abc");
        assert_eq!(
            one_node.last().expect("last").0,
            "nd-abc",
            "an id is not prettified"
        );

        // The rule, over every shape at once: no crumb points at a
        // collection with nothing after it.
        for path in [
            "/projects/db-test",
            "/projects/db-test/services/orders",
            "/projects/db-test/services/orders/settings",
            "/projects/db-test/databases/new",
            "/nodes/nd-abc",
        ] {
            for (label, href) in trail_for(Area::Projects, path) {
                let Some(href) = href else { continue };
                assert!(
                    !href.ends_with("/services") && !href.ends_with("/databases"),
                    "{label:?} links to {href}, which is not a page"
                );
            }
        }
    }

    /// A page at the root of an area shows no trail: one crumb naming
    /// the page you are looking at is furniture.
    #[test]
    fn the_root_of_an_area_has_nothing_to_show() {
        let account = Account {
            theme: Theme::System,
            id: "a".into(),
            username: "someone".into(),
            role: crate::accounts::roles::NodeRole::Admin,
            language: crate::console::language::Language::En,
        };
        let frame = Frame::new(&account, Area::Projects, &[], None, "/");
        assert_eq!(frame.trail().len(), 1);
        assert!(
            !frame
                .render(String::new())
                .render()
                .into_inner()
                .contains("class=\"crumbs\""),
            "the bar is drawn on a page that has nowhere to go back to"
        );
    }

    /// The bar names its destination once.
    ///
    /// The back control's target is always the crumb beside it — that
    /// is what a parent is — so a control that spelled its destination
    /// out printed `Projects   Projects / db-test`. Not an edge case:
    /// every page one level deep. It is an arrow with the name in its
    /// tooltip now, and this is the test that keeps it one.
    #[test]
    fn the_way_back_does_not_repeat_the_crumb_it_points_at() {
        let account = Account {
            theme: Theme::System,
            id: "a".into(),
            username: "someone".into(),
            role: crate::accounts::roles::NodeRole::Admin,
            language: crate::console::language::Language::En,
        };
        let frame = Frame::new(&account, Area::Projects, &[], None, "/projects/db-test");
        let html = frame.render(String::new()).render().into_inner();

        let (_, after) = html.split_once(r#"<nav class="crumbs""#).expect("a trail");
        let (bar, _) = after.split_once("</nav>").expect("it closes");

        assert_eq!(
            bar.matches(">Projects<").count(),
            1,
            "the parent is named twice in the bar: {bar}"
        );
        // The name is still answerable — it moved into the tooltip.
        assert!(bar.contains(r#"title="Back to Projects""#), "{bar}");
        assert!(bar.contains("db-test"), "and the page you are on: {bar}");
    }
}
