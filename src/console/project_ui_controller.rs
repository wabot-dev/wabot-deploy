//! A project's pages: what is in it, and what it is doing.

use std::sync::Arc;

use hypertext::prelude::*;

use crate::views::components::service_state::{elsewhere_of, state_cell, where_it_runs};
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::policies::access;
use crate::repositories::services;

use super::auth::{signed_in, PageQuery, SessionMiddleware};
use super::ConsoleState;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};

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
    /// Everything on this node, and what state the node is in.
    #[view("/")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(
                Redirect::found(super::auth::signed_out_destination(&self.state).await).into(),
            );
        };

        // The filter is the query, not the page: a list built from
        // everything and narrowed afterwards leaks the first time
        // somebody adds a page that forgets to narrow.
        let projects = access::projects_for(&self.state.database, &account).await?;

        // What needs somebody, read once for the page they land on. An
        // administrator's view: a member of one project has no business
        // being told about another's certificates, and every source here
        // is node-wide.
        let concerns = match account.is_admin() {
            true => super::attention::gather(&self.state)
                .await
                .unwrap_or_default(),
            false => Vec::new(),
        };

        // Whether this node originates work of its own. A node that does
        // not is somewhere resources live: it runs what it was asked to
        // run and makes nothing.
        let owns = crate::network::capability::owns(&self.state.database).await;

        layout::head("Projects");
        let frame = Frame::new(&account, Area::Projects, &projects, None, "/");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::projects_page::Projects {
            projects: &projects,
            owns,
            concerns: &concerns,
            query_error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::projects_page::render(&page)
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
        if !crate::network::capability::owns(&self.state.database).await {
            return Ok(Redirect::found("/projects").into());
        }

        let projects = access::projects_for(&self.state.database, &account).await?;

        layout::head("Create project");
        let frame = Frame::new(&account, Area::Projects, &projects, None, "/projects/new");
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::new_project_page::NewProject {
            query_error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::new_project_page::render(&page)
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
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let services = services::all(&self.state.database, Some(&project.id)).await?;
        let new_service = format!("/projects/{}/services/new", project.slug);
        let new_database = format!("/projects/{}/databases/new", project.slug);

        // Asked of containerd, one service at a time. Sequential
        // because a project has a handful of services and each answer
        // is a round trip to a local socket — a fan-out here would buy
        // milliseconds and cost the ability to read the code.
        // One question for the whole page rather than one per row: the
        // answer is a set, and asking the job store per service would
        // be the same read repeated.
        let deploying = crate::deploy::jobs::deploying(&self.state.container).await;

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
                crate::repositories::replicas::of_service(&self.state.database, &service.id)
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
                elsewhere_of(&placements),
            );
            rows.push((service, cell, here.and_then(|replica| replica.last_error)));
        }

        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let path = format!("/projects/{}", project.slug);

        layout::head(&project.name);
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            path,
        )
        .allowing(allowed);
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::project_page::ProjectView {
            project: &project,
            allowed,
            rows: &rows,
            new_service: &new_service,
            new_database: &new_database,
            query_error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::project_page::render(&page)
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
            return Ok(Redirect::found("/?error=no+such+project").into());
        };
        let here = format!("/projects/{}", project.slug);
        // A viewer may look at this project and change nothing on this
        // page, so it has nothing for them. The checks that count are
        // still on each POST.
        if !allowed.may_deploy() {
            return Ok(Redirect::found(here).into());
        }

        let tokens =
            crate::repositories::tokens::of_project(&self.state.database, &project.id).await?;
        // The stored domain, not the file's. The registry names what it
        // receives under `settings::domain`, which prefers the row the
        // console writes — so reading the file here printed a `docker
        // login` for whatever the node was called at install time, for
        // as long as nobody had edited the file to match.
        let registry_host = crate::node::settings::domain(&self.state.database, &self.state.config)
            .await
            .unwrap_or_else(|| "this node".into());

        let members = access::members(&self.state.database, &project.id).await?;
        let all_projects = access::projects_for(&self.state.database, &account).await?;
        let frame = Frame::new(
            &account,
            Area::Projects,
            &all_projects,
            Some(&project),
            format!("{here}/settings"),
        )
        .allowing(allowed);

        layout::head(&format!("{} settings", project.name));
        // The account's language, around the render and no wider:
        // the strings are read here, and nothing awaits inside.
        let page = crate::views::pages::project_settings_page::ProjectSettings {
            project: &project,
            allowed,
            here: &here,
            members: &members,
            tokens: &tokens,
            registry_host: Some(registry_host.as_str()),
            query_error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::project_settings_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}
