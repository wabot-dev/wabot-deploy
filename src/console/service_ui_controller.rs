//! A service's pages: what it is doing, what it says, and how it is set
//! up.

use std::sync::Arc;

use hypertext::prelude::*;

use crate::views::pages::service_settings_page::{Panel, Tab};
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::deploy::dns;
use crate::policies::access;
use crate::repositories::config_history;
use crate::repositories::ports;
use crate::repositories::releases;
use crate::repositories::services;
use crate::views::texts::language::t;

use super::auth::{signed_in, SessionMiddleware};
use super::ConsoleState;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};

#[derive(Debug, Deserialize, Validate)]
pub struct ServiceForm {
    pub project: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct ServicePage {
    pub project: String,
    pub service: String,
    pub error: Option<String>,
}

/// The log of one copy of a service.
#[derive(Debug, Deserialize, Validate)]
pub struct ServiceLogs {
    pub project: String,
    pub service: String,
    /// Which copy. Absent means the lowest one running here, which is
    /// the primary of a database and the only copy of most services.
    ///
    /// A **string**, parsed below. A query parameter is text, and the
    /// framework hands the field exactly what was in the URL — so a
    /// `u32` here answered `?slot=2` with `invalid type: string "2",
    /// expected u32` and a page of JSON where the log should have been.
    /// This is the console's only numeric query parameter, which is why
    /// nothing had found it.
    pub slot: Option<String>,

    /// What to look for in everything kept. Absent is no search.
    pub q: Option<String>,
    /// Which page of the matches, 1-based. A string for the reason `slot`
    /// is one.
    pub page: Option<String>,
    /// A line number to open the history around, from a match.
    pub at: Option<String>,

    /// Show everything kept rather than the end of the live file.
    ///
    /// The default is the last window of what is being written now,
    /// which is what somebody watching a deployment wants. This is for
    /// the other question — *what has this been doing* — whose answer is
    /// on the far side of one or more restarts.
    ///
    /// A string for the same reason `slot` is: a query parameter is text.
    pub all: Option<String>,
}

/// The same service, from the page that changes it.
///
/// Its own struct rather than a shared one: `checked` is produced by
/// the port form and belongs where that form is, and a page that
/// accepts a parameter nothing sends it invites somebody to send it.
#[derive(Debug, Deserialize, Validate)]
pub struct ServiceSettings {
    pub project: String,
    pub service: String,
    /// Which tab, from the path. Absent on the bare `/settings`, which
    /// redirects to the first one a service has.
    pub tab: Option<String>,
    pub error: Option<String>,
    /// What a DNS check just said, carried back from the POST that ran
    /// it. Not stored: it describes the world a second ago, and the
    /// next attempt asks again.
    pub checked: Option<String>,
}

#[injectable]
pub struct ServiceUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl ServiceUiController {
    #[view("/projects/:project/services/new")]
    #[middleware(SessionMiddleware)]
    async fn new_service(&self, query: ServiceForm) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        if !allowed.may_deploy() {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        }

        let action = format!("/projects/{}/services", project.slug);
        let back = format!("/projects/{}", project.slug);
        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/services/new", project.slug),
        )
        .allowing(allowed);

        layout::head("Create service");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::new_service_page::NewService {
            action: &action,
            back: &back,
            query_error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::new_service_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// What this service is doing, and where to reach it.
    ///
    /// Reading, and the one action that belongs with it — deploying a
    /// release. Everything that *configures* the service lives on
    /// [`Self::settings`]: the two answer different questions, and a
    /// page somebody keeps open while a deployment lands should not be
    /// a page covered in forms they are not filling in.
    #[view("/projects/:project/services/:service")]
    #[middleware(SessionMiddleware)]
    async fn service(&self, query: ServicePage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, &query.service).await?
        else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };

        let ports = ports::of_service(&self.state.database, &service.id).await?;
        let releases = releases::of_service(&self.state.database, &service.id).await?;

        // Whether each HTTPS name has a certificate yet. Asked here
        // rather than assumed: the certificate arrives seconds after
        // the route does, and a page that showed the name as ready
        // would be wrong for exactly the window somebody is looking at
        // it.
        let mut secured = std::collections::BTreeSet::new();
        for port in &ports {
            if let Some(hostname) = &port.hostname {
                if crate::edge::certs::load(&self.state.database, hostname)
                    .await
                    .ok()
                    .flatten()
                    .is_some_and(|certificate| certificate.issuer != "self-signed")
                {
                    secured.insert(hostname.clone());
                }
            }
        }
        let observed = self
            .state
            .deployer
            .observe_service(&project, &service)
            .await;
        // The address and the reason live on the copy that runs here —
        // a service is *n* of them now, and this page still shows one.
        let here =
            crate::repositories::replicas::here_for(&self.state.database, &service.id).await?;
        // Where every copy of it runs, and the nodes it could run on.
        // Both only matter when this node is the one that decides — a
        // service that arrived from elsewhere is placed from there.
        let placements =
            crate::repositories::replicas::of_service(&self.state.database, &service.id).await?;
        let nodes = crate::network::all(&self.state.database).await?;
        // Who could answer for a name, and who was asked to. Only a
        // node with an address the world can dial: sending a hostname
        // to one without would publish a name that resolves to nothing.
        //
        // This node among them, like any other. It used to be left out
        // on the grounds that it always serves its own names — which
        // read the model backwards. The only thing separating a private
        // node from a public one is whether it exposes its own address,
        // so a node can own a service and have it served from somewhere
        // else entirely, and whether it answers for its own names is a
        // decision like every other one on this page.
        //
        // The engine's own row, when this service is one. The card it
        // fills in stands in for the image field and the environment
        // editor, neither of which a managed database has.
        let engine = match service.kind.is_managed() {
            true => {
                crate::repositories::databases::of_service(&self.state.database, &service.id)
                    .await?
            }
            false => None,
        };
        // The address a connection string names. Reserved rather than
        // observed, so the page shows the same one whether or not the
        // container happens to be up — which is the whole point of
        // reserving it.
        let reserved_address = match (&engine, here.as_ref().and_then(|r| r.reserved_host)) {
            (Some(_), Some(host)) => {
                let index = crate::repositories::projects::ensure_network_index(
                    &self.state.database,
                    &project.id,
                )
                .await?;
                crate::runtime::network::ProjectNetwork::new(index)
                    .and_then(|net| net.reserved_address(host))
                    .ok()
                    .map(|address| address.to_string())
            }
            _ => None,
        };

        // Which nodes still have an instruction about this service waiting
        // to be collected. A copy elsewhere with no address is stopped, or
        // not yet told, or told and not yet reporting — and the page had
        // one word for all three.
        let queued: Vec<String> = crate::network::errand::all(&self.state.database)
            .await?
            .into_iter()
            .filter(|record| record.done_at.is_none())
            .filter(|record| {
                record.payload.get("service").and_then(|name| name.as_str())
                    == Some(service.name.as_str())
            })
            .map(|record| record.node_id)
            .collect();

        // Every name a managed database answers to, from the one function
        // that decides them — the same list the certificate is built from,
        // so the page cannot name something the certificate does not cover.
        let names = match service.kind.is_managed() {
            true => {
                crate::deploy::certificate_names(
                    &self.state.database,
                    &self.state.config,
                    &project,
                    &service,
                )
                .await
            }
            false => Vec::new(),
        };

        // Whether this node signs the certificate, which is the one thing
        // the connection string needs from that decision: with a public
        // authority it carries no `sslrootcert`, because the client's own
        // trust store is what checks it. The rest of the certificate — its
        // state, and where it comes from — is a decision, so it lives in
        // settings with the name and the port.
        let signs_here = match names.first() {
            Some(name) => matches!(
                crate::edge::policy::for_name(&self.state.database, &self.state.config, name)
                    .await
                    .renew_with,
                crate::edge::policy::RenewWith::SelfSigned
            ),
            None => false,
        };

        // What each copy on this machine is using. One read of the
        // cgroups, for the table below — a copy elsewhere is measured by
        // the node running it, and this map simply has no entry for it.
        let used = self.state.deployer.memory().await.containers;
        // Which copies the edge stopped sending to. One read of the
        // health map, for the whole table below.
        let silent = self.state.deployer.not_answering(&service).await;
        // What a restore of this database could reach. Read here rather
        // than in the card so the card stays a function of its argument.
        let window = match service.kind.is_managed() {
            true => crate::commands::backup::window(
                &self.state.config.node.data_dir,
                &format!("{}.{}", project.slug, service.slug),
                crate::commands::backup::taken(&self.state.config.node.data_dir)
                    .last()
                    .map(|(_, manifest)| manifest.taken_at),
                crate::node::settings::archiving(&self.state.database).await,
            ),
            false => crate::commands::backup::Window::NotKeeping,
        };
        // Whether any backup here holds a copy of *this* database, which
        // is what decides if there is anything to restore. The window
        // above measures the archived log, and a log with no base backup
        // to replay onto reaches nothing — that is `NoAnchor`, and it is
        // exactly the state where a restore form would only ever refuse.
        //
        // **Or the backups are somewhere else.** With a remote
        // destination there is nothing here to find — the staging copy is
        // removed when the transfer succeeds — and asking the bucket
        // "have you got one" on every page render would make an unrelated
        // page depend on somebody's network. So the card is offered, and
        // the search happens when the form is submitted: `restore_one`
        // looks here first and at the destination second.
        let elsewhere = match service.kind.is_managed() {
            true => crate::node::backups::plan(&self.state.database)
                .await
                .destination
                .filter(|text| {
                    // Anywhere that is not this node's own root: a bucket,
                    // another machine over SSH, or a second disk here.
                    // The same comparison `restore_one` makes, so the card
                    // is offered exactly when the search has a second
                    // place to look.
                    crate::commands::destination::Destination::parse(text)
                        .map(|destination| {
                            destination
                                != crate::commands::destination::Destination::Local(
                                    self.state.config.node.data_dir.join("backups"),
                                )
                        })
                        .unwrap_or(false)
                }),
            false => None,
        };
        let restorable = service.kind.is_managed()
            && (elsewhere.is_some()
                || crate::commands::backup::taken(&self.state.config.node.data_dir)
                    .iter()
                    .any(|(path, _)| {
                        path.join("volumes")
                            .join(format!("{}.{}", project.slug, service.slug))
                            .join("base.tar.gz")
                            .exists()
                    }));
        // What this service keeps, which is a row per volume and a
        // directory per copy of it.
        let volumes = crate::repositories::volumes::of_service(&self.state.database, &service.id)
            .await
            .unwrap_or_default();
        // And what each copy here holds. One walk per copy, on a page
        // somebody opened — the alternative is a figure that is only ever
        // as fresh as the last report, which for a volume on this very
        // machine would be strange.
        let disk: std::collections::BTreeMap<String, Option<u64>> = placements
            .iter()
            .filter(|replica| replica.is_here() && !replica.evicted())
            .map(|replica| {
                (
                    replica.id.clone(),
                    crate::repositories::volumes::used_by(
                        &self.state.config.node.data_dir,
                        &replica.container_id(&project.slug, &service.slug),
                    ),
                )
            })
            .collect();

        let deploying = crate::deploy::jobs::deploying(&self.state.container)
            .await
            .contains(&service.id);
        // Taken before the markup moves it: the island below names the
        // stream this page joins, and `rsx!` captures by move.
        let project_slug = project.slug.clone();
        // Built once and shared by the badge and the control, so the
        // page cannot show "running" beside a play button.
        let cell = crate::views::components::service_state::state_cell(
            &observed,
            deploying,
            crate::views::components::service_state::where_it_runs(&placements),
            crate::views::components::service_state::elsewhere_of(&placements),
        );
        // The tab, because every use of this link is about the image or
        // the tag: a bare `/settings` is a redirect now, and sending
        // somebody through one to reach a named section is a hop that
        // exists only because the caller did not say what it wanted.
        let settings = Tab::Image.path(&project.slug, &service.slug);
        let logs = format!("/projects/{}/services/{}/logs", project.slug, service.slug);
        let domain = crate::node::settings::domain(&self.state.database, &self.state.config).await;
        let exposure = crate::views::components::service_ports::exposure(
            &service,
            &project.slug,
            &ports,
            domain.as_deref(),
        );
        let push = crate::views::components::service_push::push(
            &service,
            &project.slug,
            domain.as_deref(),
        );

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/services/{}", project.slug, service.slug),
        )
        .allowing(allowed);

        layout::head(&service.name);
        let running = crate::views::components::service_running::Running {
            project_slug: &project.slug,
            service_slug: &service.slug,
            used: &used,
            disk: &disk,
            silent: &silent,
        };
        let page = crate::views::pages::service_page::ServiceDetail {
            project: &project,
            service: &service,
            allowed,
            error: query.error.as_deref(),
            logs_path: &logs,
            settings_path: &settings,
            cell: &cell,
            observed_running: matches!(observed, crate::deploy::Observed::Running { .. }),
            here: here.as_ref(),
            exposure: &exposure,
            placements: &placements,
            nodes: &nodes,
            queued: &queued,
            running,
            volumes: &volumes,
            engine: engine
                .as_ref()
                .map(|row| crate::views::pages::service_page::Engine {
                    row,
                    address: reserved_address.clone(),
                    names: &names,
                    signs_here,
                    host_port: ports.first().and_then(|port| port.host_port),
                    window: &window,
                    elsewhere: elsewhere.as_deref(),
                    restorable,
                }),
            releases: &releases,
            push: push.as_ref(),
        };
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::service_page::render(&page)
                .render()
                .into_inner()
        });

        // The project's own island, on the project's own stream. It
        // writes by service id into `data-state` and `data-address`,
        // and this page has one of each — so a page showing one service
        // costs no second endpoint and cannot drift from the list.
        //
        // Rendered first, then wrapped: `rsx!` expands to a closure
        // that captures by move, and nesting one inside the island's
        // would have both wanting the same borrows.
        let body = wabot::ui::hypertext::island(
            "project-live",
            &serde_json::json!({ "project": project_slug }),
            hypertext::Raw::dangerously_create(&body),
        )
        .render()
        .into_inner();

        Ok(frame.render(body).into_view().into())
    }

    /// What one copy of this service is saying, now.
    ///
    /// Its own page rather than a panel on the service page: a log is
    /// read for minutes at a time and wants the height, and the service
    /// page is the one somebody keeps open while a deployment lands.
    ///
    /// **Complete without JavaScript.** The window of the log renders
    /// into the page, so this works with scripting off — the stream only
    /// adds what arrives after. That is the console's rule, and a log
    /// viewer that needed a script would break in exactly the situation
    /// somebody opens one: a node that is not well.
    #[view("/projects/:project/services/:service/logs")]
    #[middleware(SessionMiddleware)]
    async fn logs(&self, query: ServiceLogs) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, &query.service).await?
        else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };

        // Every copy, read once — this used to ask twice for the same
        // rows to answer two questions about them.
        let copies: Vec<crate::entities::replica::Replica> =
            crate::repositories::replicas::of_service(&self.state.database, &service.id)
                .await?
                .into_iter()
                .filter(|replica| !replica.evicted())
                .collect();
        // The copies that run **here**, whose log is a file on this disk.
        let mut mine: Vec<u32> = copies
            .iter()
            .filter(|replica| replica.is_here())
            .map(|replica| replica.slot)
            .collect();
        mine.sort_unstable();
        // And the ones elsewhere, which this node now asks for rather
        // than telling somebody to go and look. The page said "open the
        // console of the node holding it" because there was no way to
        // ask; phase 9 built the road and this is what it was for.
        let mut away: Vec<(u32, String)> = copies
            .iter()
            .filter(|replica| !replica.is_here())
            .filter_map(|replica| {
                replica
                    .node_id
                    .clone()
                    .map(|node_id| (replica.slot, node_id))
            })
            .collect();
        away.sort_unstable_by_key(|(slot, _)| *slot);
        let mut readable: Vec<u32> = mine
            .iter()
            .copied()
            .chain(away.iter().map(|(s, _)| *s))
            .collect();
        readable.sort_unstable();

        // Anything that is not a copy of this service falls back to the
        // first one rather than erroring: a stale link or a hand-typed
        // number should show a log, not a refusal.
        let slot = query
            .slot
            .as_deref()
            .and_then(|slot| slot.parse::<u32>().ok())
            .filter(|slot| readable.contains(slot))
            .or(readable.first().copied());
        let everything = query.all.is_some();
        // Which machine the chosen copy is on. `None` is here, and every
        // reading below branches on it: a log fetched over the network
        // has no offset to follow from, nothing to search locally, and no
        // generations this node can count.
        let holder = slot.and_then(|slot| {
            away.iter()
                .find(|(one, _)| *one == slot)
                .map(|(_, node)| node.clone())
        });
        let container = slot.filter(|_| holder.is_none()).map(|slot| {
            crate::entities::replica::container_id_for(&project.slug, &service.slug, slot)
        });
        // Asked for now, and the refusal is the page's to print: a node
        // with no overlay session, a copy that has written nothing, a
        // grant withdrawn since. Every one of those is a sentence
        // somebody can act on.
        let (remote_text, remote_error) = match (&holder, slot) {
            (Some(node), Some(slot)) => match crate::network::replica_log(
                &self.state.database,
                node,
                &project.name,
                &service.name,
                slot,
                everything,
            )
            .await
            {
                Ok(text) => (Some(text), None),
                Err(reason) => (None, Some(reason)),
            },
            _ => (None, None),
        };
        // Two readings of one file, and the difference is which question
        // is being asked. The live tail is for watching; the whole kept
        // history — every rotated generation and the live file as one —
        // is for finding out what happened before the restart.
        let opened = match (&container, everything) {
            (Some(container), false) => {
                crate::deploy::logs::read_from(&self.state.config.node.data_dir, container, 0)
            }
            (Some(container), true) => {
                crate::deploy::logs::history(
                    &self.state.config.node.data_dir,
                    container,
                    512 * 1024,
                )
                .map(|text| crate::deploy::logs::Chunk {
                    text,
                    // Not a place to follow from: the history ends
                    // where the live file does, and a stream that
                    // resumed from an offset into a concatenation
                    // would be reading the wrong bytes. So the whole
                    // view does not stream, and says so.
                    next: 0,
                    restarted: false,
                })
                // A container with no file at all reads as absent
                // here too, which is the distinction the page makes
                // below.
                .or_else(|| {
                    crate::deploy::logs::read_from(&self.state.config.node.data_dir, container, 0)
                })
            }
            (None, _) => None,
        };
        // Searching, and reading a match in context. Both over the whole
        // kept history — the line somebody wants is usually before the
        // restart that made them look, which is the reason this exists at
        // all.
        use crate::views::pages::service_logs_page::PER_PAGE;
        const RADIUS: usize = 12;
        let needle = query
            .q
            .as_deref()
            .map(str::trim)
            .filter(|needle| !needle.is_empty())
            .map(str::to_string);
        let page = query
            .page
            .as_deref()
            .and_then(|page| page.parse::<usize>().ok())
            .filter(|page| *page >= 1)
            .unwrap_or(1);
        let found = match (&container, &needle) {
            (Some(container), Some(needle)) => Some(crate::deploy::logs::search(
                &self.state.config.node.data_dir,
                container,
                needle,
                (page - 1) * PER_PAGE,
                PER_PAGE,
            )),
            _ => None,
        };
        let at = query.at.as_deref().and_then(|at| at.parse::<usize>().ok());
        let window = match (&container, at) {
            (Some(container), Some(at)) => {
                crate::deploy::logs::around(&self.state.config.node.data_dir, container, at, RADIUS)
            }
            _ => Vec::new(),
        };

        // Whether there is older history to offer, so the link is absent
        // when it would lead to the same bytes.
        let has_history = container
            .as_ref()
            .map(|container| {
                crate::deploy::logs::kept_generations(&self.state.config.node.data_dir, container)
                    > 0
            })
            .unwrap_or(false);
        // Whether there is a file at all, kept apart from whether it
        // has anything in it — see the page.
        let kept = opened.is_some() || remote_text.is_some();
        let (text, from) = match (&opened, &remote_text) {
            (Some(chunk), _) => (chunk.text.clone(), chunk.next),
            // No offset: what came back is what that node chose to send,
            // and a number counted there is not a place in anything here.
            (None, Some(text)) => (text.clone(), 0),
            (None, None) => (String::new(), 0),
        };
        let elsewhere = !away.is_empty();

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("/projects/{}/services/{}/logs", project.slug, service.slug),
        )
        .allowing(allowed);

        layout::head(&format!("{} logs", service.name));
        let (project_slug, service_slug, service_name) = (
            project.slug.clone(),
            service.slug.clone(),
            service.name.clone(),
        );
        let (for_island, service_for_island) = (project_slug.clone(), service_slug.clone());
        // Read before the render takes `holder`: the island below is for
        // a copy on this machine and nothing else.
        let here = holder.is_none();
        let logs = crate::views::pages::service_logs_page::Logs {
            project_slug: &project_slug,
            service_slug: &service_slug,
            service_name: &service_name,
            readable: &readable,
            slot,
            holder: holder.as_deref(),
            elsewhere,
            everything,
            has_history,
            kept,
            text: &text,
            from,
            remote_error: remote_error.as_deref(),
            needle: needle.as_deref().unwrap_or_default(),
            page,
            found: found.as_ref(),
            at,
            window: &window,
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::service_logs_page::render(&logs)
                .render()
                .into_inner()
        });

        // One stream, for the copy being read. The island needs to know
        // which — a page that opened the wrong slot's stream would show
        // a log that did not match the one it rendered.
        let body = match slot.filter(|_| here) {
            Some(slot) => wabot::ui::hypertext::island(
                "logs-live",
                &serde_json::json!({
                    "project": for_island,
                    "service": service_for_island,
                    "slot": slot,
                    "from": from,
                    "following": crate::views::texts::language::scoped(account.language, || {
                        t("Following").to_string()
                    }),
                    "reconnecting": crate::views::texts::language::scoped(account.language, || {
                        t("Reconnecting…").to_string()
                    }),
                }),
                hypertext::Raw::dangerously_create(&body),
            )
            .render()
            .into_inner(),
            None => body,
        };

        Ok(frame.render(body).into_view().into())
    }

    /// How this service is configured, and everything that changes it.
    ///
    /// Split from [`Self::service`] because the two are read at
    /// different moments: the service page is open while a deployment
    /// lands, this one is opened to change something and left again.
    /// Every form that used to crowd that page is here, and every POST
    /// behind them comes back here rather than to the page it left.
    /// The bare path is not a page: it sends you to the first tab this
    /// service has, so there is one place each section lives and one URL
    /// that names it.
    #[view("/projects/:project/services/:service/settings")]
    #[middleware(SessionMiddleware)]
    async fn settings_root(&self, query: ServiceSettings) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, _)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, &query.service).await?
        else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };
        let first = Tab::of(&service)[0];
        Ok(Redirect::found(first.path(&project.slug, &service.slug)).into())
    }

    #[view("/projects/:project/services/:service/settings/:tab")]
    #[middleware(SessionMiddleware)]
    async fn settings(&self, query: ServiceSettings) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, &query.service).await?
        else {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        };

        let here = format!("/projects/{}/services/{}", project.slug, service.slug);
        // Every control on this page is one a viewer would be refused,
        // so they get the page they may read instead of a form that
        // lies to them. The check that matters is still on each POST.
        if !allowed.may_deploy() {
            return Ok(Redirect::found(here).into());
        }

        // A tab this kind of service does not have goes to the first one
        // it does, rather than rendering a page with nothing on it. That
        // covers a hand-typed URL and, more usefully, a link kept from
        // before a service became something else.
        let tabs = Tab::of(&service);
        let Some(tab) = query
            .tab
            .as_deref()
            .and_then(Tab::parse)
            .filter(|tab| tabs.contains(tab))
        else {
            return Ok(Redirect::found(tabs[0].path(&project.slug, &service.slug)).into());
        };

        let ports = ports::of_service(&self.state.database, &service.id).await?;

        // **Only what this tab draws.** Eight tabs used to read every
        // one's data on every render — the certificate of every port, the
        // whole environment history, the placement candidates and two
        // sweeps of the errand table — for a page that shows one of them.
        let panel = match tab {
            Tab::Database => {
                let names = crate::deploy::certificate_names(
                    &self.state.database,
                    &self.state.config,
                    &project,
                    &service,
                )
                .await;
                let certificate = match names.first() {
                    Some(name) => {
                        let facts = super::certificate_facts_for(&self.state, name).await;
                        let policy = crate::edge::policy::for_name(
                            &self.state.database,
                            &self.state.config,
                            name,
                        )
                        .await;
                        let state =
                            crate::views::components::node_certificate::CertificateState::read(
                                &facts,
                                Some(name),
                                policy.last_error.clone(),
                                self.state.certificates.phase(),
                                self.state.config.acme.disabled,
                            );
                        let cells = crate::views::components::node_certificate::certificate_cells(
                            &state,
                            &facts,
                            Some(name),
                        );
                        Some((policy, cells))
                    }
                    None => None,
                };
                Panel::Database {
                    name: names.first().cloned(),
                    actions: (
                        format!("{here}/database/name"),
                        format!("{here}/database/certificate"),
                        format!("{here}/database/publish"),
                    ),
                    certificate,
                    host_port: ports.first().and_then(|port| port.host_port),
                }
            }
            Tab::Image => {
                let domain =
                    crate::node::settings::domain(&self.state.database, &self.state.config).await;
                Panel::Image {
                    push: crate::views::components::service_push::push(
                        &service,
                        &project.slug,
                        domain.as_deref(),
                    ),
                    project_settings: format!("/projects/{}/settings", project.slug),
                }
            }
            Tab::Logs => Panel::Logs,
            Tab::Environment => Panel::Environment {
                env_text: service
                    .env
                    .iter()
                    .map(|(key, value)| format!("{key}={value}\n"))
                    .collect(),
                history: config_history::of_service(&self.state.database, &service.id).await?,
            },
            Tab::Resources => Panel::Resources,
            Tab::Network => {
                let domain =
                    crate::node::settings::domain(&self.state.database, &self.state.config).await;
                // Whether each HTTPS name has a certificate yet. Asked
                // rather than assumed: the certificate arrives seconds
                // after the route does, and a page that showed the name as
                // ready would be wrong for exactly the window somebody is
                // looking at it.
                let mut secured = std::collections::BTreeSet::new();
                let mut certificates = Vec::new();
                for port in &ports {
                    let Some(hostname) = &port.hostname else {
                        continue;
                    };
                    if crate::edge::certs::load(&self.state.database, hostname)
                        .await
                        .ok()
                        .flatten()
                        .is_some_and(|certificate| certificate.issuer != "self-signed")
                    {
                        secured.insert(hostname.clone());
                    }
                    let facts = super::certificate_facts_for(&self.state, hostname).await;
                    let policy = crate::edge::policy::for_name(
                        &self.state.database,
                        &self.state.config,
                        hostname,
                    )
                    .await;
                    let state = crate::views::components::node_certificate::CertificateState::read(
                        &facts,
                        Some(hostname),
                        // A name's own failure only. The node-wide reason
                        // belongs to the node's domain, and reading it here
                        // would blame this name for that one.
                        None,
                        self.state.certificates.phase(),
                        self.state.config.acme.disabled,
                    );
                    let cells = crate::views::components::node_certificate::certificate_cells(
                        &state,
                        &facts,
                        Some(hostname),
                    );
                    certificates.push((port.id.clone(), hostname.clone(), cells, policy));
                }
                let suffix = crate::policies::project_domain::domain_for(
                    &self.state.database,
                    &project,
                    domain.as_deref(),
                )
                .await;
                let suggestion = match &suffix {
                    Some(suffix) => Some((
                        dns::suggested_hostname(&service.slug, &project.slug, suffix),
                        dns::wildcard_works(suffix).await,
                    )),
                    None => None,
                };
                Panel::Network {
                    certificates,
                    secured,
                    domain,
                    add: format!("{here}/ports"),
                    suggestion,
                    may_publish: account.is_admin(),
                    public_nodes: crate::network::edge_candidates(&self.state.database).await?,
                    serving: crate::repositories::edges::of_service(
                        &self.state.database,
                        &service.id,
                    )
                    .await?,
                    orders: crate::network::errand::all(&self.state.database)
                        .await
                        .unwrap_or_default(),
                    ports: ports.clone(),
                }
            }
            Tab::Replicas => Panel::Replicas {
                placements: crate::repositories::replicas::of_service(
                    &self.state.database,
                    &service.id,
                )
                .await?,
                nodes: crate::network::candidates_for(
                    &self.state.database,
                    service.kind.placement_capability(),
                )
                .await?,
                silent: self.state.deployer.not_answering(&service).await,
                queued: crate::network::errand::all(&self.state.database)
                    .await?
                    .into_iter()
                    .filter(|record| record.done_at.is_none())
                    .filter(|record| {
                        record.payload.get("service").and_then(|name| name.as_str())
                            == Some(service.name.as_str())
                    })
                    .map(|record| record.node_id)
                    .collect(),
            },
            Tab::Danger => Panel::Danger,
        };

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("{here}/settings"),
        )
        .allowing(allowed);

        layout::head(&format!("{} settings", service.name));
        let page = crate::views::pages::service_settings_page::ServiceSettings {
            project: &project,
            service: &service,
            base: &here,
            tabs: &tabs,
            tab,
            panel,
            error: query.error.as_deref(),
            checked: query.checked.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::service_settings_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}
