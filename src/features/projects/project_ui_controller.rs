//! A project's pages: what is in it, and what it is doing.

use std::sync::Arc;

use hypertext::prelude::*;

use crate::features::services::service_state_component::{elsewhere_of, state_cell, where_it_runs};
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::infra::repositories::access;
use crate::infra::repositories::services;

use crate::shell::auth::{signed_in, PageQuery, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;

#[derive(Debug, Deserialize, Validate)]
pub struct ProjectQuery {
    pub project: String,
    pub error: Option<String>,
}

#[injectable]
pub struct ProjectUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl ProjectUiController {
    /// Every project this account reaches.
    #[view("/projects")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found(
                crate::shell::auth::signed_out_destination(&self.state).await,
            )
            .into());
        };

        // The filter is the query, not the page: a list built from
        // everything and narrowed afterwards leaks the first time
        // somebody adds a page that forgets to narrow.
        let projects = access::projects_for(&self.state.database, &account).await?;

        // Whether this node originates work of its own. A node that does
        // not is somewhere resources live: it runs what it was asked to
        // run and makes nothing.
        let owns = crate::infra::network::capability::owns(&self.state.database).await;

        layout::head("Projects");
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Projects,
            "/projects",
        )
        .await;
        let page = crate::features::projects::projects_page::Projects {
            projects: &projects,
            owns,
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::projects::projects_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/new")]
    #[middleware(SessionMiddleware)]
    async fn new_project(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        // A hand-typed URL, or a link kept from before the switch was
        // turned off. The list says why; a form that cannot submit does
        // not.
        if !crate::infra::network::capability::owns(&self.state.database).await {
            return Ok(Redirect::found("/projects").into());
        }

        layout::head("Create project");
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Projects,
            "/projects/new",
        )
        .await;
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::features::projects::new_project_page::NewProject {
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::projects::new_project_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// One project: its services, and what each of them is doing.
    #[view("/projects/:project")]
    #[middleware(SessionMiddleware)]
    async fn project(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };

        // "Not yours" answers exactly like "not there". Telling them
        // apart turns the project list into something anybody can
        // enumerate by guessing names.
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/projects?error=no+such+project").into());
        };
        let services = services::all(&self.state.database, Some(&project.id)).await?;
        let new_service = format!("/projects/{}/services/new", project.slug);
        let new_database = format!("/projects/{}/databases/new", project.slug);
        let new_store = crate::infra::node::settings::buckets_enabled(&self.state.database)
            .await
            .then(|| format!("/projects/{}/stores/new", project.slug));

        // Asked of containerd, one service at a time. Sequential
        // because a project has a handful of services and each answer
        // is a round trip to a local socket — a fan-out here would buy
        // milliseconds and cost the ability to read the code.
        // One question for the whole page rather than one per row: the
        // answer is a set, and asking the job store per service would
        // be the same read repeated.
        let deploying = crate::infra::deploy::jobs::deploying(&self.state.container).await;

        let mut rows = Vec::with_capacity(services.len());
        for service in services {
            let observed = self
                .state
                .deployer
                .observe_service(&project, &service)
                .await;
            // The address and the reason live on the copy that runs
            // here — a service is *n* of them now, and this row still
            // shows one.
            let placements =
                crate::infra::repositories::replicas::of_service(&self.state.database, &service.id)
                    .await?;
            let here = placements
                .iter()
                .find(|replica| replica.is_here() && !replica.evicted())
                .cloned();
            let where_it_runs = where_it_runs(&placements);
            // The same value the stream sends, so the first paint and
            // every update after it cannot disagree about which action
            // applies or what the badge says.
            let cell = state_cell(
                &observed,
                deploying.contains(&service.id),
                where_it_runs,
                elsewhere_of(service.desired_state, &placements),
            );
            rows.push((
                service,
                cell,
                here.and_then(|replica| replica.failure().map(str::to_string)),
            ));
        }

        let path = format!("/projects/{}", project.slug);

        layout::head(&project.name);
        let frame = Frame::load(&self.state.database, &account, Section::Projects, path).await;
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::features::projects::project_page::ProjectView {
            project: &project,
            allowed,
            rows: &rows,
            new_service: &new_service,
            new_database: &new_database,
            new_store: new_store.as_deref(),
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::projects::project_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// Who is in this project, and what they may do.
    ///
    /// Its own page rather than a card under the services, because the
    /// two are read at different times by different people: the list of
    /// services is what somebody opens twenty times a day, and access
    /// is what somebody changes when a person joins or leaves.
    ///
    /// Readable by anyone who can see the project — knowing who else is
    /// here is not privileged — while the forms need administration.
    /// Where the people page went.
    ///
    /// Kept as a redirect rather than removed: it was a nav item for
    /// months, so it is in somebody's history and somebody's bookmark,
    /// and a 404 would tell them the feature was gone when it had only
    /// moved one page over.
    #[view("/projects/:project/people")]
    #[middleware(SessionMiddleware)]
    async fn project_people(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        Ok(Redirect::found(format!("/projects/{}/settings", query.project)).into())
    }

    /// What the project is configured with, and how to destroy it.
    ///
    /// Push tokens and the danger zone: the two things nobody opens a
    /// project page *for*, and the two that were hardest to find under
    /// a table of services.
    #[view("/projects/:project/settings")]
    #[middleware(SessionMiddleware)]
    async fn project_settings(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/projects?error=no+such+project").into());
        };
        let here = format!("/projects/{}", project.slug);
        // A viewer may look at this project and change nothing on this
        // page, so it has nothing for them. The checks that count are
        // still on each POST.
        if !allowed.may_deploy() {
            return Ok(Redirect::found(here).into());
        }

        let tokens =
            crate::infra::repositories::tokens::of_project(&self.state.database, &project.id)
                .await?;
        // The stored domain, not the file's. The registry names what it
        // receives under `settings::domain`, which prefers the row the
        // console writes — so reading the file here printed a `docker
        // login` for whatever the node was called at install time, for
        // as long as nobody had edited the file to match.
        let registry_host = crate::infra::node::authority(
            crate::infra::registry::name_of(&self.state.database, &project)
                .await
                .as_deref(),
            self.state.config.edge.https_port,
        );
        let registry_domain = crate::infra::repositories::projects::registry_domain(
            &self.state.database,
            &project.id,
        )
        .await?;
        let registry_allowlist = crate::infra::repositories::allowlists::of(
            &self.state.database,
            &crate::domain::values::allowlist::Door::ProjectRegistry(project.id.clone()),
        )
        .await?;

        let members = access::members(&self.state.database, &project.id).await?;
        let frame = Frame::load(
            &self.state.database,
            &account,
            Section::Projects,
            format!("{here}/settings"),
        )
        .await;

        layout::head(&format!("{} settings", project.name));
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::features::projects::project_settings_page::ProjectSettings {
            project: &project,
            allowed,
            here: &here,
            members: &members,
            tokens: &tokens,
            registry_host: Some(registry_host.as_str()),
            registry_domain: registry_domain.as_deref(),
            registry_allowlist: &registry_allowlist,
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::projects::project_settings_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/:project/settings/environment")]
    #[middleware(SessionMiddleware)]
    async fn project_env_groups(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/projects?error=no+such+project").into());
        };
        if !allowed.may_deploy() {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        }
        let names = service_names(&self.state.database, &project.id).await?;
        let mut groups = Vec::new();
        for group in
            crate::infra::repositories::env_groups::of_project(&self.state.database, &project.id)
                .await?
        {
            let users = crate::infra::repositories::env_groups::services_using(
                &self.state.database,
                &group.id,
            )
            .await?;
            groups.push((group, named(&names, &users)));
        }

        let path = crate::features::projects::project_settings_tabs_component::ProjectSettingsTab::Environment
            .path(&project.slug);
        let frame = Frame::load(&self.state.database, &account, Section::Projects, path).await;
        layout::head(&format!("{} settings", project.name));
        let page = crate::features::projects::project_env_groups_page::ProjectEnvGroups {
            project: &project,
            allowed,
            groups: &groups,
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::projects::project_env_groups_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/:project/settings/allowlists")]
    #[middleware(SessionMiddleware)]
    async fn project_allowlists(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/projects?error=no+such+project").into());
        };
        if !allowed.may_deploy() {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        }
        let names = service_names(&self.state.database, &project.id).await?;
        let mut sets = Vec::new();
        for set in crate::infra::repositories::allowlist_sets::of_project(
            &self.state.database,
            &project.id,
        )
        .await?
        {
            let users = crate::infra::repositories::allowlist_sets::services_using(
                &self.state.database,
                &set.id,
            )
            .await?;
            sets.push((set, named(&names, &users)));
        }

        let path = crate::features::projects::project_settings_tabs_component::ProjectSettingsTab::Allowlists
            .path(&project.slug);
        let frame = Frame::load(&self.state.database, &account, Section::Projects, path).await;
        layout::head(&format!("{} settings", project.name));
        let page = crate::features::projects::project_allowlists_page::ProjectAllowlists {
            project: &project,
            allowed,
            sets: &sets,
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::projects::project_allowlists_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/:project/settings/internet")]
    #[middleware(SessionMiddleware)]
    async fn project_egress(&self, query: ProjectQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let Some((project, allowed)) =
            access::find_project(&self.state.database, &account, &query.project).await?
        else {
            return Ok(Redirect::found("/projects?error=no+such+project").into());
        };
        if !allowed.may_deploy() {
            return Ok(Redirect::found(format!("/projects/{}", project.slug)).into());
        }
        let destinations =
            crate::infra::repositories::egress::of_project(&self.state.database, &project.id)
                .await?;
        let refused =
            crate::infra::repositories::egress::refused(&self.state.database, &project.id, 20)
                .await?;
        let exits = exit_choices(&self.state, &project).await?;

        let path = crate::features::projects::project_settings_tabs_component::ProjectSettingsTab::Internet
            .path(&project.slug);
        let frame = Frame::load(&self.state.database, &account, Section::Projects, path).await;
        layout::head(&format!("{} settings", project.name));
        let page = crate::features::projects::project_egress_page::ProjectEgress {
            project: &project,
            allowed,
            destinations: &destinations,
            exits: &exits,
            refused: &refused,
            query_error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::projects::project_egress_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}

async fn service_names(
    database: &wabot::sqlite::SqliteDatabase,
    project_id: &str,
) -> Result<Vec<(String, String)>, crate::domain::errors::PlatformError> {
    Ok(services::all(database, Some(project_id))
        .await?
        .into_iter()
        .map(|service| (service.id, service.name))
        .collect())
}

fn named(names: &[(String, String)], ids: &[String]) -> Vec<String> {
    ids.iter()
        .filter_map(|id| {
            names
                .iter()
                .find(|(known, _)| known == id)
                .map(|(_, name)| name.clone())
        })
        .collect()
}

/// The nodes the owner may choose from, ticked where chosen, and any node
/// still chosen that no longer lets this one name it.
async fn exit_choices(
    state: &ConsoleState,
    project: &crate::domain::entities::project::Project,
) -> UiResult<Vec<crate::features::projects::project_egress_page::ExitChoice>> {
    use crate::features::projects::project_egress_page::ExitChoice;
    let chosen = crate::infra::repositories::egress::exits_of(&state.database, &project.id).await?;
    let candidates = match project.is_ours() {
        true => state
            .deployer
            .exit_candidates()
            .await
            .map_err(|error| UiError::Internal(error.to_string()))?,
        false => Vec::new(),
    };
    let mut choices: Vec<ExitChoice> = candidates
        .iter()
        .map(|candidate| ExitChoice {
            node_id: Some(candidate.node_id.clone()),
            name: candidate.name.clone(),
            is_self: candidate.is_self,
            chosen: chosen.contains(&candidate.address),
        })
        .collect();
    let nodes = crate::infra::network::all(&state.database)
        .await
        .unwrap_or_default();
    for address in chosen.iter().filter(|address| {
        !candidates
            .iter()
            .any(|candidate| candidate.address == **address)
    }) {
        let known = nodes
            .iter()
            .find(|node| node.overlay_ip.as_deref() == Some(address.to_string().as_str()));
        choices.push(ExitChoice {
            node_id: None,
            name: known.map_or_else(|| address.to_string(), |node| node.name.clone()),
            is_self: known.is_some_and(|node| node.is_self),
            chosen: true,
        });
    }
    Ok(choices)
}
