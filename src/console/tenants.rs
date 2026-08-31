//! Organisations, and what each of them may take.
//!
//! ## The node's operator only
//!
//! Making an organisation, giving it a domain and setting its ceiling
//! are decisions about the *machine* — how it is shared out — so they
//! sit beside the node's own settings and ask `shares_the_machine_out`,
//! which a tenant's administrator is not.
//!
//! ## And inside one, only who answers for it
//!
//! The organisations page used to carry an invitation form with a role
//! and a project selector, which made the operator an administrator
//! inside a frontier that is not theirs. What they decide is *who
//! answers for this organisation*; members, project membership and
//! everything else is the organisation's own, from its own console.
//!
//! The one reach that stays is removing an administrator, and it earns
//! itself: an organisation whose last one has gone cannot invite a
//! replacement, so without it the only way to hand one over would be to
//! delete it with its projects inside.
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
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;
use wabot::ui::hypertext::IntoView;

use super::auth::{
    back_with_error, field, read_form, see_other, signed_in, PageQuery, SessionMiddleware,
};
use super::ConsoleState;
use crate::accounts::roles::NodeRole;
use crate::policies::access;
use crate::repositories::tenants;
use crate::views::layouts::layout;
use crate::views::layouts::shell::{Area, Frame};
use crate::views::pages::node_user_organisation_settings_page::{OrganisationTab, Panel};

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
pub struct TenantPages {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[ui_controller("/", app)]
impl TenantPages {
    /// Every organisation on this node, as tiles that open one.
    #[view("/organisations")]
    #[middleware(SessionMiddleware)]
    async fn index(&self, query: PageQuery) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        if !crate::authz::node_user::shares_the_machine_out(&account) {
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
            let people = crate::accounts::of_scope(&self.state.database, Some(&tenant.id))
                .await
                .unwrap_or_default()
                .len();
            cards.push(crate::views::pages::node_user_organisations_page::Card {
                tenant,
                projects,
                people,
            });
        }

        let frame = Frame::new(&account, Area::Settings, &[], None, "/organisations");
        let page = crate::views::pages::node_user_organisations_page::Organisations {
            cards: &cards,
            error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_organisations_page::render(&page)
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
        if !crate::authz::node_user::shares_the_machine_out(&account) {
            return Ok(Redirect::found("/").into());
        }
        let Some(tenant) = tenants::find(&self.state.database, &query.tenant).await? else {
            return Ok(Redirect::found("/organisations").into());
        };

        let projects = access::of_tenant(&self.state.database, &tenant.id)
            .await
            .unwrap_or_default();
        let owners = owners_of(&self.state, &tenant.id).await;
        let charged = crate::policies::quotas::charged(&self.state.database, &tenant.id)
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
        let page = crate::views::pages::node_user_organisation_page::Organisation {
            tenant: &tenant,
            owners: &owners,
            projects: &projects,
            charged: &charged,
            error: query.error.as_deref(),
        };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_organisation_page::render(&page)
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
        if !crate::authz::node_user::shares_the_machine_out(&account) {
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
                people: crate::accounts::of_scope(&self.state.database, Some(&tenant.id))
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
            crate::views::pages::node_user_organisation_settings_page::OrganisationSettings {
                tenant: &tenant,
                tab,
                panel,
                error: query.error.as_deref(),
            };
        let body = crate::views::texts::language::scoped(account.language, || {
            crate::views::pages::node_user_organisation_settings_page::render(&page)
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
async fn owners_of(state: &ConsoleState, tenant_id: &str) -> Vec<crate::accounts::Account> {
    crate::accounts::of_scope(&state.database, Some(tenant_id))
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|person| person.role == NodeRole::Admin)
        .collect()
}

#[injectable]
pub struct TenantApi {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

impl TenantApi {
    /// The operator, or the response that sends whoever else away.
    /// Boxed, because a `Response` is 128 bytes and every handler here
    /// returns this on its happy path too — the compiler sizes a
    /// `Result` by its larger arm.
    fn operator(&self) -> Result<crate::accounts::Account, Box<Response>> {
        match signed_in(&self.auth) {
            None => Err(Box::new(see_other("/sign-in"))),
            Some(account) => match crate::authz::node_user::shares_the_machine_out(&account) {
                true => Ok(account),
                false => Err(Box::new(see_other("/"))),
            },
        }
    }
}

#[rest_controller("/")]
impl TenantApi {
    #[post("/organisations")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn create(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::usecases::node_user_create_organisation::run(
            &self.state.database,
            &account,
            field(&form, "name"),
        )
        .await
        {
            // Straight to the one just made: the operator's next move is
            // to give it a domain and hand it to somebody, and both are
            // on its own page.
            Ok(tenant) => Ok(see_other(&OrganisationTab::Domain.path(&tenant.id))),
            Err(reason) => Ok(back_with_error("/organisations", &reason)),
        }
    }

    #[post("/organisations/:tenant/domain")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_domain(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Domain.path(id);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::usecases::node_user_set_organisation_domain::run(
            &self.state.database,
            &self.state.config,
            &self.state.deployer,
            &account,
            id,
            field(&form, "domain"),
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/organisations/:tenant/quota")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_quota(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Quota.path(id);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        match crate::usecases::node_user_set_organisation_quota::run(
            &self.state.database,
            &account,
            id,
            field(&form, "memory"),
            field(&form, "cpu"),
        )
        .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    /// Hand the organisation to somebody.
    #[post("/organisations/:tenant/owners")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn invite_owner(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Owner.path(id);

        let asked = crate::dtos::requests::invite_owner::InviteOwner {
            organisation: id.to_string(),
        };
        if let Err(reason) = asked.validate() {
            return Ok(back_with_error(&here, &reason.to_string()));
        }

        let invited = match crate::usecases::node_user_invite_owner::run(
            &self.state.database,
            &account,
            &asked,
        )
        .await
        {
            Ok(invited) => invited,
            Err(reason) => return Ok(back_with_error(&here, &reason)),
        };

        // On the organisation's own name: the node's would sign them up
        // and bounce them on their first click.
        let link = format!(
            "{}/join/{}",
            self.state.base_url_for(Some(&invited.organisation)).await,
            invited.token
        );
        // A page of its own, and the nonce rather than the link: an
        // invitation is a secret, and it used to travel in the query
        // string where the address bar, the history and every refresh
        // could read it.
        let nonce = self.state.reveals.stash(link);
        Ok(see_other(&format!(
            "/organisations/{id}/shown?{}",
            form_urlencoded::Serializer::new(String::new())
                .append_pair("nonce", &nonce)
                .finish()
        )))
    }

    #[post("/organisations/:tenant/owners/:account/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn remove_owner(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let segments = super::auth::segments(&path);
        let (Some(tenant), Some(person)) = (segments.get(1), segments.get(3)) else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Owner.path(tenant);

        match crate::usecases::node_user_remove_owner::run(&self.state.database, &account, person)
            .await
        {
            Ok(()) => Ok(see_other(&here)),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }

    #[post("/organisations/:tenant/delete")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn delete(&self, request: Request) -> RestResult<Response> {
        let account = match self.operator() {
            Ok(account) => account,
            Err(response) => return Ok(*response),
        };
        let path = request.uri().path().to_string();
        let Some(id) = super::auth::segments(&path).get(1).cloned() else {
            return Ok(see_other("/organisations"));
        };
        let here = OrganisationTab::Danger.path(id);
        let form = match read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        // **The name, typed.** The same confirmation a service asks for,
        // and this one takes more with it: every project, every person
        // and every copy running anywhere.
        let Some(tenant) = tenants::find(&self.state.database, id).await? else {
            return Ok(see_other("/organisations"));
        };
        if field(&form, "confirm") != tenant.slug {
            return Ok(back_with_error(
                &here,
                "type the organisation's slug to confirm",
            ));
        }

        match crate::usecases::node_user_delete_organisation::run(
            &self.state.database,
            &self.state.deployer,
            &account,
            id,
        )
        .await
        {
            Ok(()) => Ok(see_other("/organisations")),
            Err(reason) => Ok(back_with_error(&here, &reason)),
        }
    }
}
