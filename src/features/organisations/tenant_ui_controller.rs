//! Organisations, and what each of them may take.
//!
//! ## The node's operator only
//!
//! Making an organisation, giving it a domain and setting its ceiling
//! are decisions about the *machine* — how it is shared out — so they
//! sit beside the node's own settings and ask `shares_the_machine_out`,
//! which a tenant's administrator is not.
//!
//! ## The domain is the frontier
//!
//! A tenant's console answers on its own hostname, and `edge::dispatch`
//! stamps the scope on the request before any handler runs — so the
//! narrowing is the route rather than something thirty queries have to
//! remember. Setting the domain here is therefore what brings a tenant's
//! console into existence at all; without one it has projects and
//! nowhere to administer them from.

use std::sync::Arc;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::domain::values::roles::NodeRole;
use crate::features::organisations::node_user_organisation_settings_page::{
    OrganisationTab, Panel,
};
use crate::infra::repositories::access;
use crate::infra::repositories::tenants;
use crate::shell::auth::{signed_in, PageQuery, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Area, Frame};
use crate::shell::state::ConsoleState;

#[derive(Debug, Deserialize, Validate, Default)]
pub struct OrganisationPage {
    pub tenant: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Validate, Default)]
pub struct OrganisationSettingsPage {
    pub tenant: String,
    /// Which tab, from the path. Absent on the bare `/settings`, which
    /// sends you to the first one.
    pub tab: Option<String>,
    pub error: Option<String>,
}

#[injectable]
pub struct TenantUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl TenantUiController {
    /// Every organisation on this node, as tiles that open one.
    #[view("/organisations")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::shares_the_machine_out(&account) {
            return Ok(Redirect::found("/").into());
        }

        // The head, or the page renders as a document with no
        // stylesheet — nav, content and all, in Times New Roman. It is
        // what every other view does first and the one thing a new page
        // has nothing reminding it of.
        layout::head("Organisations");

        let mut cards = Vec::new();
        for tenant in tenants::all(&self.state.database).await? {
            let projects = access::of_tenant(&self.state.database, &tenant.id)
                .await
                .unwrap_or_default()
                .len();
            let people = crate::infra::accounts::of_scope(&self.state.database, Some(&tenant.id))
                .await
                .unwrap_or_default()
                .len();
            cards.push(
                crate::features::organisations::node_user_organisations_page::Card {
                    tenant,
                    projects,
                    people,
                },
            );
        }

        let frame = Frame::new(&account, Area::Settings, &[], None, "/organisations");
        let page = crate::features::organisations::node_user_organisations_page::Organisations {
            cards: &cards,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::organisations::node_user_organisations_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// One organisation: what the machine is answerable for.
    #[view("/organisations/:tenant")]
    #[middleware(SessionMiddleware)]
    async fn detail(&self, query: OrganisationPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::shares_the_machine_out(&account) {
            return Ok(Redirect::found("/").into());
        }
        let Some(tenant) = tenants::find(&self.state.database, &query.tenant).await? else {
            return Ok(Redirect::found("/organisations").into());
        };

        let projects = access::of_tenant(&self.state.database, &tenant.id)
            .await
            .unwrap_or_default();
        let owners = owners_of(&self.state, &tenant.id).await;
        let charged = crate::infra::deploy::quotas::charged(&self.state.database, &tenant.id)
            .await
            .unwrap_or_default();

        layout::head(&tenant.name);
        let frame = Frame::new(
            &account,
            Area::Settings,
            &[],
            None,
            format!("/organisations/{}", tenant.id),
        );
        let page = crate::features::organisations::node_user_organisation_page::Organisation {
            tenant: &tenant,
            owners: &owners,
            projects: &projects,
            charged: &charged,
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::organisations::node_user_organisation_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }

    /// The bare path is not a page: it names the first tab.
    #[view("/organisations/:tenant/settings")]
    #[middleware(SessionMiddleware)]
    async fn settings_root(&self, query: OrganisationPage) -> UiResult<ViewOutcome> {
        Ok(Redirect::found(OrganisationTab::Domain.path(&query.tenant)).into())
    }

    #[view("/organisations/:tenant/settings/:tab")]
    #[middleware(SessionMiddleware)]
    async fn settings(&self, query: OrganisationSettingsPage) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::domain::authz::node_user::shares_the_machine_out(&account) {
            return Ok(Redirect::found("/").into());
        }
        let Some(tenant) = tenants::find(&self.state.database, &query.tenant).await? else {
            return Ok(Redirect::found("/organisations").into());
        };
        // A word that is not a tab goes to the first one, which covers a
        // hand-typed URL and a link kept from before there were tabs.
        let Some(tab) = query.tab.as_deref().and_then(OrganisationTab::parse) else {
            return Ok(Redirect::found(OrganisationTab::Domain.path(&tenant.id)).into());
        };

        // Only what this tab draws — the same rule the node's own
        // settings follow, and here it is the difference between two
        // reads and none.
        let owners = match tab {
            OrganisationTab::Owner => owners_of(&self.state, &tenant.id).await,
            _ => Vec::new(),
        };
        let panel = match tab {
            OrganisationTab::Domain => Panel::Domain,
            OrganisationTab::Owner => Panel::Owner { owners: &owners },
            OrganisationTab::Quota => Panel::Quota,
            OrganisationTab::Danger => Panel::Danger {
                projects: access::of_tenant(&self.state.database, &tenant.id)
                    .await
                    .unwrap_or_default()
                    .len(),
                people: crate::infra::accounts::of_scope(&self.state.database, Some(&tenant.id))
                    .await
                    .unwrap_or_default()
                    .len(),
            },
        };

        layout::head("Settings");
        let frame = Frame::new(
            &account,
            Area::Settings,
            &[],
            None,
            format!("/organisations/{}", tenant.id),
        );
        let page =
            crate::features::organisations::node_user_organisation_settings_page::OrganisationSettings {
                tenant: &tenant,
                tab,
                panel,
                error: query.error.as_deref(),
            };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::organisations::node_user_organisation_settings_page::render(&page)
                .render()
                .into_inner()
        });

        Ok(frame.render(body).into_view().into())
    }
}

/// Who answers for an organisation: its administrators.
///
/// A function rather than a repository call spelled three times, because
/// what "answers for it" means is a decision — `NodeRole::Admin` inside
/// the frontier — and three copies of it is three places to disagree.
async fn owners_of(
    state: &ConsoleState,
    tenant_id: &str,
) -> Vec<crate::domain::entities::account::Account> {
    crate::infra::accounts::of_scope(&state.database, Some(tenant_id))
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|person| person.role == NodeRole::Admin)
        .collect()
}
