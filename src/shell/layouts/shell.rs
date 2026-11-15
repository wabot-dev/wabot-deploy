//! The frame every signed-in page sits in.
//!
//! Three columns, each answering one question and none of them changing
//! shape: the rail says what kinds of thing there are, the list says
//! which ones, and the page is the one somebody chose. The header holds
//! what belongs to the session — a way to jump by name, the language, the
//! theme, the way out — and no navigation.
//!
//! It replaced a header that picked an area and a sidebar that changed
//! with it, which left two bars sharing one job by a rule nothing on the
//! screen stated. See `docs/navigation.md`.
//!
//! Rendered on the server with no client router, so "which link is
//! current" is decided here from the request's path.

use hypertext::prelude::*;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::values::theme::Theme;

use crate::shell::assets;
use crate::shell::texts::language::t;

impl Theme {
    /// Empty for `System`: no attribute means the media query decides.
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

    /// The icon for the state it is *in*: one picturing its destination
    /// reads as a status to everybody who does not know it is a button.
    fn icon(self) -> &'static str {
        match self {
            Self::System => MONITOR,
            Self::Light => SUN,
            Self::Dark => MOON,
        }
    }

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

/// The rail's five, drawn here on the grid and stroke the others use. The
/// design system ships no icons, so these are the console's own; each
/// sits above its word, because an icon alone is a guess.
const ICON_STATUS: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 12h-4l-3 9L9 3l-3 9H2"/></svg>"#;

const ICON_PROJECTS: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.7-.9L9.6 3.9A2 2 0 0 0 7.9 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z"/></svg>"#;

const ICON_NODES: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="2" y="2" width="20" height="8" rx="2"/><rect x="2" y="14" width="20" height="8" rx="2"/><path d="M6 6h.01M6 18h.01"/></svg>"#;

const ICON_ORGANISATIONS: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="4" y="2" width="16" height="20" rx="2"/><path d="M9 22v-4h6v4M8 6h.01M12 6h.01M16 6h.01M8 10h.01M12 10h.01M16 10h.01M8 14h.01M12 14h.01M16 14h.01"/></svg>"#;

const ICON_PEOPLE: &str = r#"<svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M16 21v-2a4 4 0 0 0-4-4H6a4 4 0 0 0-4 4v2"/><circle cx="9" cy="7" r="4"/><path d="M22 21v-2a4 4 0 0 0-3-3.87M16 3.13a4 4 0 0 1 0 7.75"/></svg>"#;

const ICON_SEARCH: &str = r#"<svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;

/// The kinds of thing this console holds, which is what the rail lists.
///
/// Things, and no "Settings": what can be changed about a node, a
/// project, a service or an organisation is a tab of that thing's own
/// page, so the word means one thing wherever it appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Status,
    Projects,
    Nodes,
    Organisations,
    People,
    /// The page a search lands on, which is under none of them.
    Search,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Self::Status => t("Status"),
            Self::Projects => t("Projects"),
            Self::Nodes => t("Nodes"),
            Self::Organisations => t("Organisations"),
            Self::People => t("People"),
            Self::Search => t("Go to"),
        }
    }

    fn href(self) -> &'static str {
        match self {
            Self::Status => "/status",
            Self::Projects => "/projects",
            Self::Nodes => "/nodes",
            Self::Organisations => "/organisations",
            Self::People => "/people",
            Self::Search => "/go",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Status => ICON_STATUS,
            Self::Projects => ICON_PROJECTS,
            Self::Nodes => ICON_NODES,
            Self::Organisations => ICON_ORGANISATIONS,
            Self::People | Self::Search => ICON_PEOPLE,
        }
    }
}

/// What the list says about one of its entries, beyond its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// A copy of it would not start. Read off the rows, never asked of
    /// the runtime: the list is drawn on every page, and a green dot here
    /// would be a health check nobody ran.
    Failed,
    ThisNode,
}

/// One line of the list: a thing somebody can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub href: String,
    pub label: String,
    pub mark: Option<Mark>,
    /// Inside the entry above it: a service under its project.
    pub nested: bool,
}

impl Entry {
    pub fn new(href: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            label: label.into(),
            mark: None,
            nested: false,
        }
    }

    pub fn marked(mut self, mark: Option<Mark>) -> Self {
        self.mark = mark;
        self
    }

    pub fn nested(mut self) -> Self {
        self.nested = true;
        self
    }
}

/// Everything the frame needs to draw itself.
pub struct Frame {
    pub username: String,
    /// Whether they run the machine, which is what the rail's node-wide
    /// sections are offered on.
    pub admin: bool,
    /// Whether they administer whatever scope they are in. **Not the same
    /// as `admin`**: an organisation's administrator manages its people
    /// and nothing about the machine.
    pub scope_admin: bool,
    pub section: Section,
    pub path: String,
    pub list: Vec<Entry>,
    /// This machine among the nodes, as `(id, name)`: its settings live
    /// at `/node`, a path that names nobody.
    pub this_node: Option<(String, String)>,
    /// The page wants the list's width for itself.
    pub wide: bool,
    pub theme: Theme,
    pub language: crate::domain::values::language::Language,
}

/// The last segment of a path when it names an action or a tab rather
/// than a thing.
fn verb(segment: &str) -> Option<String> {
    Some(match segment {
        "new" => t("New").to_string(),
        "settings" => t("Settings").to_string(),
        "logs" => t("Logs").to_string(),
        "connections" => t("Connections").to_string(),
        "version" => t("Version").to_string(),
        "join" => t("Join a network").to_string(),
        "shown" => t("Shown once").to_string(),
        _ => return None,
    })
}

/// What to call a collection nested under a thing, when it is a page.
///
/// `None` for the ones that are not — `/projects/x/services` — because a
/// crumb linking to one is a link to a 404, and for the rail's own, whose
/// crumb is the root of every trail under them.
fn collection(segment: &str) -> Option<String> {
    Some(match segment {
        "zones" => t("DNS").to_string(),
        _ => return None,
    })
}

impl Frame {
    /// The frame with nothing in its list, for a page that has none and
    /// for a test.
    pub fn new(account: &Account, section: Section, path: impl Into<String>) -> Self {
        Self {
            username: account.username.clone(),
            admin: account.is_admin(),
            scope_admin: account.administers_scope(),
            section,
            path: path.into(),
            list: Vec::new(),
            this_node: None,
            wide: false,
            theme: account.theme,
            language: account.language,
        }
    }

    /// The frame for a page, with the list its section shows.
    ///
    /// It reads what it draws, so a page hands over nothing but where it
    /// is. A list that could not be read is an empty column and never a
    /// page that does not render: this console is opened when the node is
    /// unwell.
    pub async fn load(
        database: &SqliteDatabase,
        account: &Account,
        section: Section,
        path: impl Into<String>,
    ) -> Self {
        let mut frame = Self::new(account, section, path);
        match section {
            Section::Projects => frame.list = projects_list(database, account, &frame.path).await,
            Section::Nodes if frame.admin => {
                let nodes = crate::infra::network::all(database)
                    .await
                    .unwrap_or_default();
                frame.this_node = nodes
                    .iter()
                    .find(|node| node.is_self)
                    .map(|node| (node.id.clone(), node.name.clone()));
                frame.list = nodes
                    .iter()
                    .map(|node| {
                        Entry::new(format!("/nodes/{}", node.id), &node.name)
                            .marked(node.is_self.then_some(Mark::ThisNode))
                    })
                    .collect();
            }
            Section::Organisations if frame.admin => {
                frame.list = crate::infra::repositories::tenants::all(database)
                    .await
                    .unwrap_or_default()
                    .iter()
                    .map(|tenant| Entry::new(format!("/organisations/{}", tenant.id), &tenant.name))
                    .collect();
            }
            _ => {}
        }
        frame
    }

    /// For a page that needs the room: a log is read by the line, and a
    /// line cut short by a column of names is the wrong trade.
    pub fn wide(mut self) -> Self {
        self.wide = true;
        self
    }

    /// Where the brand goes: what this account came to look at.
    fn home(&self) -> &'static str {
        match self.admin {
            true => Section::Status.href(),
            false => Section::Projects.href(),
        }
    }

    /// What this account's rail offers. A section they would be turned
    /// away from is not listed: the rail is where somebody looks for what
    /// there is.
    fn sections(&self) -> Vec<Section> {
        match (self.admin, self.scope_admin) {
            (true, _) => vec![
                Section::Status,
                Section::Projects,
                Section::Nodes,
                Section::Organisations,
                Section::People,
            ],
            (false, true) => vec![Section::Projects, Section::People],
            (false, false) => vec![Section::Projects],
        }
    }

    fn shows_list(&self) -> bool {
        !self.wide && !self.list.is_empty()
    }

    /// The path the list should light for this page. This machine's
    /// settings and version are at `/node`, and belong to its entry.
    fn listed_path(&self) -> String {
        match (&self.this_node, self.path.as_str()) {
            (Some((id, _)), path) if path == "/node" || path.starts_with("/node/") => {
                format!("/nodes/{id}")
            }
            _ => self.path.clone(),
        }
    }

    /// The whole frame, with the page's own markup inside it.
    ///
    /// Takes the body already rendered: nesting the page's `rsx!` inside
    /// the frame's would have both borrowing the same values at once, and
    /// the closure `rsx!` builds captures by move.
    pub fn render(&self, inner: String) -> impl Renderable + '_ {
        let language = self.language;
        let top =
            crate::shell::texts::language::scoped(language, || self.topbar().render().into_inner());
        let rail =
            crate::shell::texts::language::scoped(language, || self.rail().render().into_inner());
        let list = match self.shows_list() {
            true => crate::shell::texts::language::scoped(language, || {
                self.listing().render().into_inner()
            }),
            false => String::new(),
        };
        let crumbs =
            crate::shell::texts::language::scoped(language, || self.crumbs().render().into_inner());
        let body = match (self.shows_list(), self.wide) {
            (true, _) => "app-body has-list",
            (false, true) => "app-body wide",
            (false, false) => "app-body",
        };
        rsx! {
            <div class="app-shell" data-theme=(self.theme.attribute())>
                (hypertext::Raw::dangerously_create(&top))
                <div class=(body)>
                    (hypertext::Raw::dangerously_create(&rail))
                    (hypertext::Raw::dangerously_create(&list))
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
    /// Derived from the path rather than passed in: a trail every view
    /// has to remember to supply is a trail some view does not have. The
    /// way back is the crumb before the last, so the control is an arrow
    /// with the name in its tooltip — spelling it out printed the parent
    /// twice on every page one level deep.
    fn crumbs(&self) -> impl Renderable + '_ {
        let trail = self.trail();
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

    /// The trail, as `(label, href)` from the section to this page.
    ///
    /// The path is read in **`collection/id` pairs**, so `services` is not
    /// a crumb of its own — `/projects/x/services` is not a page. What is
    /// left over at the end is a verb or a tab, which is the current page.
    fn trail(&self) -> Vec<(String, Option<String>)> {
        let mut trail = vec![(
            self.section.label().to_string(),
            Some(self.section.href().to_string()),
        )];
        let segments: Vec<&str> = self.path.split('/').filter(|s| !s.is_empty()).collect();

        if segments.first() == Some(&"node") {
            self.this_machine(&segments, &mut trail);
            return trail;
        }

        let mut index = 0;
        while index < segments.len() {
            let segment = segments[index];
            if let Some(label) = verb(segment) {
                trail.push((label, None));
                return trail;
            }
            let id = match segments.get(index + 1) {
                Some(id) if verb(id).is_none() => id,
                _ => {
                    if let Some(label) = collection(segment) {
                        trail.push((label, Some(format!("/{}", segments[..=index].join("/")))));
                    }
                    index += 1;
                    continue;
                }
            };
            if let Some(label) = collection(segment) {
                trail.push((label, Some(format!("/{}", segments[..=index].join("/")))));
            }
            trail.push((
                self.label_for(segment, id),
                Some(format!("/{}", segments[..=index + 1].join("/"))),
            ));
            index += 2;
        }
        trail
    }

    /// `/node/…`, which is this machine's settings and its version: under
    /// its own name among the nodes, though the path carries no id.
    fn this_machine(&self, segments: &[&str], trail: &mut Vec<(String, Option<String>)>) {
        if let Some((id, name)) = &self.this_node {
            trail.push((name.clone(), Some(format!("/nodes/{id}"))));
        }
        match segments.get(1) {
            Some(&"version") => {
                trail.push((t("Version").to_string(), Some("/node/version".to_string())));
                if let Some(tag) = segments.get(2) {
                    trail.push((tag.to_string(), None));
                }
            }
            _ => trail.push((t("Settings").to_string(), None)),
        }
    }

    /// What to call one thing in the trail: its name when the list knows
    /// it, because the name is what somebody typed, and otherwise its id
    /// unchanged — prettifying one is how a page stops matching `ctr`.
    fn label_for(&self, collection: &str, id: &str) -> String {
        if collection == "nodes" && id == "invite" {
            return t("Invite a node").to_string();
        }
        let href = format!("/{collection}/{id}");
        self.list
            .iter()
            .find(|entry| entry.href == href)
            .map(|entry| entry.label.clone())
            .unwrap_or_else(|| id.to_string())
    }

    fn topbar(&self) -> impl Renderable + '_ {
        rsx! {
            <header class="topbar">
                <a class="brand" href=(self.home())>
                    <img
                        src=(format!("{}/wabot-logo.png", assets::MOUNT))
                        alt="Wabot" width="20" height="20">
                    <span>("wabot-deploy")</span>
                </a>
                (self.search())
                <div class="topbar-right">
                    (self.languages())
                    (self.toggle())
                    // A form, not a link: signing out changes state, and
                    // a GET that changes state is one a prefetcher fires.
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

    /// Jump to a thing by its name.
    ///
    /// A form that submits with scripting off; the island around it only
    /// adds the keyboard's way to the field.
    fn search(&self) -> impl Renderable + '_ {
        let form = rsx! {
            <form method="get" action="/go" class="go" role="search">
                (hypertext::Raw::dangerously_create(ICON_SEARCH))
                <input type="search" name="q" autocomplete="off"
                       placeholder=(t("Go to…")) aria-label=(t("Go to"))>
            </form>
        }
        .render()
        .into_inner();
        wabot::ui::hypertext::island(
            "go",
            &serde_json::json!({}),
            hypertext::Raw::dangerously_create(&form),
        )
    }

    /// A word rather than a flag: a flag is a country and a language is
    /// not one.
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
    /// While an account is on `System` the **server cannot know** which
    /// the machine is, and the icon shows the state you are in — so both
    /// are rendered and the media query picks. After one press the answer
    /// is stored and one button is drawn.
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

    fn rail(&self) -> impl Renderable + '_ {
        rsx! {
            <nav class="rail" aria-label=(t("Sections"))>
                <div class="rail-inner">
                    @for section in self.sections() {
                        @if section == self.section {
                            <a href=(section.href()) class="active" aria-current="true">
                                (hypertext::Raw::dangerously_create(section.icon()))
                                <span>(section.label())</span>
                            </a>
                        } @else {
                            <a href=(section.href())>
                                (hypertext::Raw::dangerously_create(section.icon()))
                                <span>(section.label())</span>
                            </a>
                        }
                    }
                </div>
            </nav>
        }
    }

    fn listing(&self) -> impl Renderable + '_ {
        let here = self.listed_path();
        let inside = self
            .list
            .iter()
            .any(|entry| entry.nested && within(&here, &entry.href));
        rsx! {
            <aside class="side-list" aria-label=(self.section.label())>
                // The column carries the background the height of the
                // page; the box inside it is what stays put.
                <div class="side-inner">
                    <p class="side-label">(self.section.label())</p>
                    <nav>
                        @for entry in &self.list {
                            <a href=(&entry.href) class=(entry_class(entry, &here, inside))>
                                <span class="side-name">(&entry.label)</span>
                                @match entry.mark {
                                    Some(Mark::Failed) => {
                                        <span class="side-mark failed">
                                            <span class="dot dot-danger"></span>(t("Failed"))
                                        </span>
                                    }
                                    Some(Mark::ThisNode) => {
                                        <span class="side-mark">(t("this node"))</span>
                                    }
                                    None => {}
                                }
                            </a>
                        }
                    </nav>
                </div>
            </aside>
        }
    }
}

/// The projects this account reaches, with the services of the one the
/// page is in.
///
/// Only that one is opened: an operator hosting several organisations
/// has more services than a column holds, and the ones somebody jumps
/// between are the ones beside the page they are on.
async fn projects_list(database: &SqliteDatabase, account: &Account, path: &str) -> Vec<Entry> {
    let projects = crate::infra::repositories::access::projects_for(database, account)
        .await
        .unwrap_or_default();
    let services = crate::infra::repositories::services::all(database, None)
        .await
        .unwrap_or_default();
    let failed: Vec<String> = crate::infra::repositories::replicas::here(database)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|replica| replica.failure().is_some())
        .map(|replica| replica.service_id)
        .collect();

    let mut list = Vec::new();
    for project in &projects {
        let href = format!("/projects/{}", project.slug);
        let open = within(path, &href);
        let inside: Vec<_> = services
            .iter()
            .filter(|service| service.project_id == project.id)
            .collect();
        let any_failed = inside.iter().any(|service| failed.contains(&service.id));
        list.push(
            Entry::new(&href, &project.name).marked((any_failed && !open).then_some(Mark::Failed)),
        );
        if open {
            for service in inside {
                list.push(
                    Entry::new(format!("{href}/services/{}", service.slug), &service.name)
                        .marked(failed.contains(&service.id).then_some(Mark::Failed))
                        .nested(),
                );
            }
        }
    }
    list
}

/// Whether `path` is `href` or a page under it.
fn within(path: &str, href: &str) -> bool {
    path == href
        || path
            .strip_prefix(href)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// How one entry of the list is drawn for the page being shown.
///
/// A project with one of its services open is where you are without being
/// the page you are on, so it is marked open and the service is the one
/// that is lit — two lit rows would be two answers to "where am I".
fn entry_class(entry: &Entry, here: &str, inside: bool) -> &'static str {
    match (entry.nested, within(here, &entry.href), inside) {
        (true, true, _) => "nested active",
        (true, false, _) => "nested",
        (false, true, true) => "open",
        (false, true, false) => "active",
        (false, false, _) => "",
    }
}

/// The frame's own layout. Colour, type and radii come from the design
/// system's tokens — this is arrangement. No borders, no shadows:
/// separation is background contrast.
pub const CSS: &str = r#"
/* Block flow rather than a grid of two rows. A sticky element cannot
   leave its containing block, and a grid item's block is its grid
   area — as row one of this shell, the topbar had 3.25rem of room to
   move in and scrolled away with the page. */
.app-shell { min-height: 100vh; }
.app-body {
  display: grid;
  grid-template-columns: 5.5rem minmax(0, 1fr);
  min-height: calc(100vh - 3.25rem);
}
.app-body.has-list { grid-template-columns: 5.5rem 15rem minmax(0, 1fr); }

/* A page title is a label, not a headline. The design system's `h1`
   is a marketing size, and these are hostnames: one step further down,
   and a long one breaks rather than pushing the page sideways. */
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
.app-body.wide > main { max-width: 90rem; }

.topbar {
  position: sticky;
  top: 0;
  z-index: 3;
  /* Block flow does not set this, and a topbar that sizes to its
     content is a topbar that changes height between pages. */
  height: 3.25rem;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  gap: var(--sp-5);
  padding: 0 var(--sp-5);
  background: rgb(var(--c-bg-sunken));
}
.topbar .brand {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: var(--sp-2);
  font-weight: 600;
  letter-spacing: -0.02em;
  color: rgb(var(--c-fg));
  text-decoration: none;
}
.topbar-right {
  flex: none;
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: var(--sp-3);
}

/* Jump by name. The field is the form: the icon sits inside its box,
   which is why the box is the form's and the input has none. */
.go {
  display: flex;
  align-items: center;
  gap: var(--sp-2);
  width: min(22rem, 100%);
  min-width: 0;
  padding: 0 var(--sp-3);
  border-radius: var(--r-md);
  background: rgb(var(--c-bg-raised));
  color: rgb(var(--c-fg-muted));
}
.go svg { flex: none; }
.go input,
[data-theme='dark'] .go input {
  flex: 1;
  min-width: 0;
  padding: 0.4rem 0;
  font-size: var(--fs-sm);
  background: transparent;
  box-shadow: none;
}
.go:focus-within { box-shadow: 0 0 0 2px rgb(var(--c-focus) / 0.45); }

/* The rail: what kinds of thing there are. Each is its icon over its
   word, because an icon alone is a guess. */
.rail { background: rgb(var(--c-bg-sunken)); padding: var(--sp-3) var(--sp-2); }
.rail-inner {
  position: sticky;
  top: calc(3.25rem + var(--sp-3));
  display: flex;
  flex-direction: column;
  gap: var(--sp-1);
}
.rail a {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--sp-1);
  padding: var(--sp-2) 2px;
  border-radius: var(--r-md);
  font-size: var(--fs-xs);
  line-height: 1.2;
  text-align: center;
  color: rgb(var(--c-fg-muted));
  text-decoration: none;
}
.rail a.active {
  background: rgb(var(--c-bg-raised));
  color: rgb(var(--c-fg));
  font-weight: 500;
}

/* The list: which ones. The column stays stretched so its background
   runs the height of the page; the box inside it is what sticks.
   Sticking the column itself needs `align-self: start`, which collapses
   it to the height of the links and takes the background with it. */
.side-list { background: rgb(var(--c-bg-raised)); padding: var(--sp-5) var(--sp-3); }
.side-inner {
  position: sticky;
  top: calc(3.25rem + var(--sp-5));
  display: flex;
  flex-direction: column;
  gap: var(--sp-2);
  /* A list longer than the viewport scrolls in place rather than
     pushing its end below the fold, where sticky cannot help. */
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
.side-list nav { display: flex; flex-direction: column; gap: 2px; }
.side-list nav a {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--sp-2);
  padding: 0.45rem var(--sp-3);
  border-radius: var(--r-md);
  font-size: var(--fs-sm);
  color: rgb(var(--c-fg-body));
  text-decoration: none;
}
.side-list nav a.nested { padding-left: var(--sp-6); }
.side-list nav a.open { color: rgb(var(--c-fg)); font-weight: 500; }
.side-list nav a.active {
  background: rgb(var(--c-bg-sunken));
  color: rgb(var(--c-fg));
  font-weight: 500;
}
.side-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.side-mark {
  flex: none;
  display: inline-flex;
  align-items: center;
  gap: var(--sp-1);
  font-size: var(--fs-xs);
  font-weight: 400;
  color: rgb(var(--c-fg-muted));
}
.side-mark.failed { color: rgb(var(--c-danger-fg)); }

/* The tabs of one thing: what it is, what is particular to it, and its
   settings last. A strip with the current one raised, so it does not
   read as the row of chips a settings page has under it. */
.object-tabs {
  display: flex;
  gap: 2px;
  width: fit-content;
  max-width: 100%;
  padding: 3px;
  border-radius: var(--r-md);
  background: rgb(var(--c-bg-sunken));
  overflow-x: auto;
}
.object-tabs a {
  flex: none;
  padding: 0.4rem var(--sp-4);
  border-radius: 6px;
  font-size: var(--fs-sm);
  font-weight: 500;
  color: rgb(var(--c-fg-muted));
  text-decoration: none;
  white-space: nowrap;
}
.object-tabs a.active {
  background: rgb(var(--c-bg-raised));
  color: rgb(var(--c-fg));
}

/* A laptop: the list gives its width back to the page, and the trail
   above the title is the way to what it listed. */
@media (max-width: 70rem) {
  .app-body.has-list { grid-template-columns: 5.5rem minmax(0, 1fr); }
  .side-list { display: none; }
}

/* A phone: the rail goes to the bottom, under the thumb. Block flow
   again, for the same reason as `.app-shell`. */
@media (max-width: 48rem) {
  .app-body,
  .app-body.has-list { display: block; min-height: 0; }
  .rail {
    position: fixed;
    left: 0;
    right: 0;
    bottom: 0;
    z-index: 3;
    padding: var(--sp-1) var(--sp-2) calc(var(--sp-1) + env(safe-area-inset-bottom, 0px));
  }
  .rail-inner {
    position: static;
    flex-direction: row;
    justify-content: space-around;
  }
  .rail a { flex: 1 1 0; min-width: 0; }
  .app-body > main { padding: var(--sp-6) var(--sp-4) calc(var(--sp-12) + 4rem); }
  .topbar { gap: var(--sp-3); padding: 0 var(--sp-3); }
  .topbar .brand span { display: none; }
  .topbar-right { gap: var(--sp-1); }
  /* A table wider than a phone scrolls inside itself. Left to the page
     it pushes everything sideways, and the bar fixed to the bottom is
     then wider than the screen it is fixed to. */
  .app-body > main table { display: block; overflow-x: auto; }
  /* A title and its buttons take a line each when one line will not
     hold both. Squeezed onto one, the title broke inside a word. */
  .app-body > main .split { flex-wrap: wrap; }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn account(role: crate::domain::values::roles::NodeRole, tenant: Option<&str>) -> Account {
        Account {
            theme: Theme::System,
            id: "a".into(),
            username: "someone".into(),
            tenant_id: tenant.map(str::to_string),
            role,
            language: crate::domain::values::language::Language::En,
        }
    }

    fn operator() -> Account {
        account(crate::domain::values::roles::NodeRole::Admin, None)
    }

    fn rendered(frame: &Frame) -> String {
        frame.render(String::new()).render().into_inner()
    }

    fn between<'a>(html: &'a str, open: &str, close: &str) -> &'a str {
        let (_, after) = html
            .split_once(open)
            .unwrap_or_else(|| panic!("no {open} in {html}"));
        after
            .split_once(close)
            .unwrap_or_else(|| panic!("{open} never closes"))
            .0
    }

    /// The complaint this frame answers: two bars shared the navigation
    /// by a rule nothing on the screen stated. The header holds what
    /// belongs to the session and links to no section.
    #[test]
    fn the_header_does_not_navigate() {
        let html = rendered(&Frame::new(&operator(), Section::Projects, "/projects"));
        let header = between(&html, "<header", "</header>");
        for section in [
            Section::Projects,
            Section::Nodes,
            Section::Organisations,
            Section::People,
        ] {
            assert!(
                !header.contains(&format!("href=\"{}\"", section.href())),
                "the header links to {}: {header}",
                section.href()
            );
        }
        assert!(header.contains("action=\"/go\""), "{header}");
    }

    /// The rail is the same on every page: it lights where you are and
    /// offers everything else, whichever section the page is in.
    #[test]
    fn the_rail_does_not_change_with_the_page() {
        let links = |section, path: &str| {
            let html = rendered(&Frame::new(&operator(), section, path));
            between(&html, "<nav class=\"rail\"", "</nav>")
                .matches("href=")
                .count()
        };
        assert_eq!(links(Section::Status, "/status"), 5);
        assert_eq!(links(Section::Projects, "/projects/x/services/y"), 5);
        assert_eq!(links(Section::Nodes, "/node/domain"), 5);
    }

    /// What somebody would be turned away from is not offered. An
    /// organisation's administrator has projects and people; a member has
    /// projects; neither is shown the machine's sections.
    #[test]
    fn the_rail_offers_only_what_the_account_can_open() {
        use crate::domain::values::roles::NodeRole;
        let rail = |account: &Account| {
            let html = rendered(&Frame::new(account, Section::Projects, "/projects"));
            between(&html, "<nav class=\"rail\"", "</nav>").to_string()
        };

        let tenant_admin = rail(&account(NodeRole::Admin, Some("t-1")));
        assert!(
            tenant_admin.contains("href=\"/projects\""),
            "{tenant_admin}"
        );
        assert!(tenant_admin.contains("href=\"/people\""), "{tenant_admin}");
        for hidden in ["/status", "/nodes", "/organisations"] {
            assert!(
                !tenant_admin.contains(&format!("href=\"{hidden}\"")),
                "an organisation's administrator is offered {hidden}"
            );
        }

        let member = rail(&account(NodeRole::Member, None));
        assert_eq!(member.matches("href=").count(), 1, "{member}");
        assert!(member.contains("href=\"/projects\""), "{member}");
    }

    /// The brand goes to what the account came to look at: the node for
    /// whoever runs it, their projects for everybody else.
    #[test]
    fn the_brand_goes_home() {
        use crate::domain::values::roles::NodeRole;
        let brand = |account: &Account| {
            let html = rendered(&Frame::new(account, Section::Projects, "/projects"));
            between(&html, "<a class=\"brand\"", ">").to_string()
        };
        assert!(brand(&operator()).contains("href=\"/status\""));
        assert!(brand(&account(NodeRole::Member, None)).contains("href=\"/projects\""));
        assert!(brand(&account(NodeRole::Admin, Some("t-1"))).contains("href=\"/projects\""));
    }

    fn with_list(section: Section, path: &str, list: Vec<Entry>) -> Frame {
        let mut frame = Frame::new(&operator(), section, path);
        frame.list = list;
        frame
    }

    /// `.side-inner` is what stays put while the page scrolls — the
    /// column around it only carries the background. A wrapper with no
    /// content of its own reads as removable, and removing it costs
    /// nothing visible until somebody scrolls.
    #[test]
    fn the_list_sits_in_the_box_that_sticks() {
        let frame = with_list(
            Section::Projects,
            "/projects/shop",
            vec![Entry::new("/projects/shop", "Shop")],
        );
        let html = rendered(&frame);
        let inside = between(&html, r#"class="side-inner""#, "</aside>");
        assert!(inside.contains(r#"href="/projects/shop""#), "{inside}");
    }

    /// One row is lit. On a service's page that is the service; its
    /// project is open around it, and lighting both would be two answers
    /// to "where am I".
    #[test]
    fn one_entry_of_the_list_is_where_you_are() {
        let list = vec![
            Entry::new("/projects/shop", "Shop"),
            Entry::new("/projects/shop/services/orders", "orders").nested(),
            Entry::new("/projects/shop/services/ledger", "ledger").nested(),
            Entry::new("/projects/shopping", "Shopping"),
        ];
        let lit = |path: &str| {
            let html = rendered(&with_list(Section::Projects, path, list.clone()));
            let column = between(&html, "<aside", "</aside>").to_string();
            (
                column.matches("active").count(),
                column.contains(r#"class="open""#),
            )
        };
        assert_eq!(lit("/projects/shop"), (1, false));
        assert_eq!(lit("/projects/shop/settings"), (1, false));
        assert_eq!(lit("/projects/shop/services/orders/logs"), (1, true));
        assert_eq!(lit("/projects/shop/services/new"), (1, false));
        assert_eq!(lit("/projects/new"), (0, false));
    }

    /// `/projects/shop` is not under `/projects/sho`, and `/projects/shop`
    /// does not light `/projects/shopping`.
    #[test]
    fn a_page_is_under_an_entry_by_whole_segments() {
        assert!(within("/projects/shop", "/projects/shop"));
        assert!(within("/projects/shop/settings", "/projects/shop"));
        assert!(!within("/projects/shopping", "/projects/shop"));
        assert!(!within("/projects", "/projects/shop"));
    }

    /// A state in the list is a word beside its dot. A colour alone says
    /// nothing to somebody who cannot tell two of them apart.
    #[test]
    fn a_mark_in_the_list_is_a_word() {
        let frame = with_list(
            Section::Projects,
            "/projects",
            vec![Entry::new("/projects/shop", "Shop").marked(Some(Mark::Failed))],
        );
        let html = rendered(&frame);
        let mark = between(&html, r#"class="side-mark failed""#, "</a>");
        assert!(mark.contains("Failed"), "{mark}");
    }

    /// A page that asked for the width gets it: the list is not drawn,
    /// and the trail is what leads back to what it listed.
    #[test]
    fn a_wide_page_has_no_list() {
        let frame = with_list(
            Section::Projects,
            "/projects/shop/services/orders/logs",
            vec![Entry::new("/projects/shop", "Shop")],
        )
        .wide();
        let html = rendered(&frame);
        assert!(!html.contains("<aside"), "{html}");
        assert!(html.contains(r#"class="app-body wide""#), "{html}");
        assert!(html.contains(r#"<nav class="crumbs""#), "{html}");
    }

    /// This machine's settings are at `/node`, which names nobody — and
    /// they are its own entry's page, so that entry is the one lit and
    /// the trail runs through its name.
    #[test]
    fn this_machines_settings_belong_to_its_entry() {
        let mut frame = with_list(
            Section::Nodes,
            "/node/domain",
            vec![
                Entry::new("/nodes/nd-self", "node-0").marked(Some(Mark::ThisNode)),
                Entry::new("/nodes/nd-other", "node-1"),
            ],
        );
        frame.this_node = Some(("nd-self".into(), "node-0".into()));

        assert_eq!(
            frame.trail(),
            vec![
                ("Nodes".to_string(), Some("/nodes".to_string())),
                ("node-0".to_string(), Some("/nodes/nd-self".to_string())),
                ("Settings".to_string(), None),
            ]
        );
        let html = rendered(&frame);
        let column = between(&html, "<aside", "</aside>");
        let lit = between(column, r#"class="active""#, "</a>");
        assert!(lit.contains("node-0"), "{column}");

        frame.path = "/node/version/v0.9.0".into();
        let release = frame.trail();
        assert_eq!(
            release[2],
            ("Version".to_string(), Some("/node/version".to_string()))
        );
        assert_eq!(release[3], ("v0.9.0".to_string(), None));
    }

    /// The shapes a trail takes, and the one it must never take.
    ///
    /// `/projects/x/services` is not a page, so `services` cannot be a
    /// crumb of its own — the path is read in `collection/id` pairs for
    /// exactly that reason, and a crumb linking to a 404 is worse than
    /// no crumb.
    #[test]
    fn a_trail_never_links_to_a_path_that_is_not_a_page() {
        let trail_for = |section, path: &str| {
            with_list(
                section,
                path,
                vec![
                    Entry::new("/projects/db-test", "Database test"),
                    Entry::new("/nodes/nd-abc", "node-1"),
                ],
            )
            .trail()
        };

        let project_page = trail_for(Section::Projects, "/projects/db-test");
        assert_eq!(
            project_page,
            vec![
                ("Projects".to_string(), Some("/projects".to_string())),
                (
                    "Database test".to_string(),
                    Some("/projects/db-test".to_string())
                ),
            ]
        );

        let service = trail_for(Section::Projects, "/projects/db-test/services/orders");
        assert_eq!(service.last().expect("a last crumb").0, "orders");
        assert_eq!(service.len(), 3, "{service:?}");

        let settings = trail_for(
            Section::Projects,
            "/projects/db-test/services/orders/settings/image",
        );
        assert_eq!(
            settings.last().expect("last"),
            &("Settings".to_string(), None)
        );

        let new = trail_for(Section::Projects, "/projects/new");
        assert_eq!(new.last().expect("last"), &("New".to_string(), None));

        let zone = trail_for(Section::Projects, "/projects/db-test/zones/example.com");
        assert_eq!(
            zone[2],
            (
                "DNS".to_string(),
                Some("/projects/db-test/zones".to_string())
            )
        );

        // A node is named, like a project: the list knows it.
        let one_node = trail_for(Section::Nodes, "/nodes/nd-abc/connections");
        assert_eq!(
            one_node,
            vec![
                ("Nodes".to_string(), Some("/nodes".to_string())),
                ("node-1".to_string(), Some("/nodes/nd-abc".to_string())),
                ("Connections".to_string(), None),
            ]
        );
        // And an id the list does not know is not prettified.
        assert_eq!(
            trail_for(Section::Nodes, "/nodes/nd-gone")[1].0,
            "nd-gone".to_string()
        );

        let invite = trail_for(Section::Nodes, "/nodes/invite/shown");
        assert_eq!(invite[1].0, "Invite a node");
        assert_eq!(invite.last().expect("last").0, "Shown once");
        assert_eq!(
            trail_for(Section::Nodes, "/nodes/join")
                .last()
                .expect("last")
                .0,
            "Join a network"
        );

        // What the arrow reads: the crumb before the last, which has to
        // be somewhere it can go.
        for (section, path) in [
            (Section::Nodes, "/nodes/nd-abc"),
            (Section::Projects, "/projects/db-test/services/orders/logs"),
            (Section::People, "/people/shown"),
            (Section::Organisations, "/organisations/t-1/settings/domain"),
        ] {
            let trail = trail_for(section, path);
            let parent = &trail[trail.len() - 2];
            assert!(parent.1.is_some(), "{path} has no way back: {trail:?}");
        }

        for path in [
            "/projects/db-test",
            "/projects/db-test/services/orders",
            "/projects/db-test/services/orders/settings",
            "/projects/db-test/databases/new",
        ] {
            for (label, href) in trail_for(Section::Projects, path) {
                let Some(href) = href else { continue };
                assert!(
                    !href.ends_with("/services") && !href.ends_with("/databases"),
                    "{label:?} links to {href}, which is not a page"
                );
            }
        }
    }

    /// A page at the root of a section shows no trail: one crumb naming
    /// the page you are looking at is furniture.
    #[test]
    fn the_root_of_a_section_has_nothing_to_show() {
        for (section, path) in [
            (Section::Status, "/status"),
            (Section::Projects, "/projects"),
            (Section::Nodes, "/nodes"),
            (Section::People, "/people"),
        ] {
            let frame = Frame::new(&operator(), section, path);
            assert_eq!(frame.trail().len(), 1, "{path}");
            assert!(
                !rendered(&frame).contains("class=\"crumbs\""),
                "{path} draws a bar with nowhere to go back to"
            );
        }
    }

    /// The bar names its destination once.
    ///
    /// The back control's target is always the crumb beside it — that
    /// is what a parent is — so a control that spelled its destination
    /// out printed `Projects   Projects / db-test`. It is an arrow with
    /// the name in its tooltip, and this is the test that keeps it one.
    #[test]
    fn the_way_back_does_not_repeat_the_crumb_it_points_at() {
        let frame = Frame::new(&operator(), Section::Projects, "/projects/db-test");
        let html = rendered(&frame);
        let bar = between(&html, r#"<nav class="crumbs""#, "</nav>");

        assert_eq!(
            bar.matches(">Projects<").count(),
            1,
            "the parent is named twice in the bar: {bar}"
        );
        assert!(bar.contains(r#"title="Back to Projects""#), "{bar}");
        assert!(bar.contains("db-test"), "and the page you are on: {bar}");
    }
}
