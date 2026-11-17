//! A store's Buckets tab, and one bucket's page.

use std::sync::Arc;
use std::time::Duration;

use hypertext::prelude::*;
use serde::Deserialize;
use wabot::prelude::*;
use wabot::ui::hypertext::IntoView;

use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::domain::values::roles::Access;
use crate::infra::garage::Usage;
use crate::infra::repositories::{access, object_stores, ports, services};
use crate::shell::auth::{signed_in, SessionMiddleware};
use crate::shell::layouts::layout;
use crate::shell::layouts::shell::{Frame, Section};
use crate::shell::state::ConsoleState;

/// How long a page waits for the store to say what its buckets hold. The
/// page renders without the numbers rather than hang on a store that is
/// starting.
const ASKING: Duration = Duration::from_secs(3);

#[derive(Debug, Deserialize, Validate)]
pub struct BucketsForm {
    pub project: String,
    pub service: String,
    pub error: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
pub struct BucketForm {
    pub project: String,
    pub service: String,
    pub bucket: String,
    pub error: Option<String>,
}

#[injectable]
pub struct BucketUiController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

impl BucketUiController {
    async fn store(
        &self,
        account: &crate::domain::entities::account::Account,
        project: &str,
        service: &str,
    ) -> UiResult<Result<(Project, Access, Service), String>> {
        let Some((project, allowed)) =
            access::find_project(&self.state.database, account, project).await?
        else {
            return Ok(Err("/projects?error=no+such+project".into()));
        };
        let Some(service) =
            services::in_project(&self.state.database, &project.id, service).await?
        else {
            return Ok(Err(format!("/projects/{}", project.slug)));
        };
        if !service.kind.is_store() {
            return Ok(Err(format!(
                "/projects/{}/services/{}",
                project.slug, service.slug
            )));
        }
        Ok(Ok((project, allowed, service)))
    }

    async fn store_hostname(&self, service: &Service) -> Option<String> {
        ports::of_service(&self.state.database, &service.id)
            .await
            .ok()?
            .into_iter()
            .find(|port| port.container_port == crate::domain::policies::garage::S3_PORT)?
            .hostname
    }

    fn address(&self, hostname: &str) -> String {
        format!(
            "https://{}",
            crate::infra::node::authority(Some(hostname), self.state.config.edge.https_port)
        )
    }

    async fn endpoint(&self, service: &Service) -> Option<String> {
        let hostname = self.store_hostname(service).await?;
        Some(self.address(&hostname))
    }

    async fn usage_of(
        &self,
        service: &Service,
        names: &[String],
        holder: Option<&crate::infra::deploy::stores::Holder>,
    ) -> Option<Vec<Option<Usage>>> {
        if holder.is_some() {
            let reported = object_stores::measured(&self.state.database, &service.id)
                .await
                .ok()?;
            if reported.is_empty() && !names.is_empty() {
                return None;
            }
            return Some(
                names
                    .iter()
                    .map(|name| {
                        reported
                            .iter()
                            .find(|measured| &measured.name == name)
                            .map(Usage::reported)
                    })
                    .collect(),
            );
        }
        let admin = crate::infra::garage::admin_for(&self.state.database, &service.id)
            .await
            .ok()?;
        tokio::time::timeout(ASKING, async {
            let mut found = Vec::with_capacity(names.len());
            for name in names {
                found.push(admin.usage(name).await.ok().flatten());
            }
            found
        })
        .await
        .ok()
    }
}

#[ui_controller("/", app)]
impl BucketUiController {
    #[view("/projects/:project/services/:service/buckets")]
    #[middleware(SessionMiddleware)]
    async fn buckets(&self, query: BucketsForm) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let (project, allowed, service) =
            match self.store(&account, &query.project, &query.service).await? {
                Ok(found) => found,
                Err(elsewhere) => return Ok(Redirect::found(elsewhere).into()),
            };
        let rows = object_stores::buckets(&self.state.database, &service.id).await?;
        let names: Vec<String> = rows.iter().map(|b| b.name.as_str().to_string()).collect();
        let holder =
            crate::infra::deploy::stores::held_elsewhere(&self.state.database, &service.id).await;
        let usage = self.usage_of(&service, &names, holder.as_ref()).await;
        let running = usage.is_some();
        let listed: Vec<_> = rows
            .into_iter()
            .zip(
                usage
                    .unwrap_or_default()
                    .into_iter()
                    .chain(std::iter::repeat_with(|| None)),
            )
            .collect();
        let reserved = object_stores::reserved(&self.state.database, &service.id).await?;
        let endpoint = self.endpoint(&service).await;
        let default_name = format!("{}-restored", service.slug);
        let domain = crate::infra::node::settings::domain(&self.state.database).await;
        let derived = crate::infra::repositories::project_domain::domain_for(
            &self.state.database,
            &project,
            domain.as_deref(),
        )
        .await
        .map(|suffix| {
            crate::infra::deploy::dns::suggested_hostname("<name>", &project.slug, &suffix)
        });
        let elsewhere = crate::infra::backup::plan::elsewhere(
            &self.state.database,
            &self.state.config.node.data_dir,
        )
        .await;
        let container = format!("{}.{}", project.slug, service.slug);
        let restorable = allowed.may_deploy()
            && service.is_ours()
            && holder.is_none()
            && (elsewhere.is_some()
                || crate::infra::backup::taken(&self.state.config.node.data_dir)
                    .iter()
                    .any(|(path, _)| {
                        path.join("volumes")
                            .join(&container)
                            .join(crate::domain::policies::garage::DATA_VOLUME)
                            .exists()
                    }));

        let path = format!(
            "/projects/{}/services/{}/buckets",
            project.slug, service.slug
        );
        layout::head(&service.name);
        let frame = Frame::load(&self.state.database, &account, Section::Projects, path).await;
        let page = crate::features::buckets::buckets_page::Buckets {
            project: &project,
            service: &service,
            may_deploy: allowed.may_deploy(),
            error: query.error.as_deref(),
            endpoint: endpoint.as_deref(),
            buckets: &listed,
            reserved,
            disk: service.disk_limit,
            running,
            runs_on: holder.as_ref().map(|holder| holder.name.as_str()),
            restore: restorable.then_some(crate::features::buckets::buckets_page::Restore {
                default_name: &default_name,
                derived: derived.as_deref(),
                elsewhere: elsewhere.as_deref(),
            }),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::buckets::buckets_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }

    #[view("/projects/:project/services/:service/buckets/:bucket")]
    #[middleware(SessionMiddleware)]
    async fn bucket(&self, query: BucketForm) -> UiResult<ViewOutcome> {
        let Some(account) = signed_in(&self.auth) else {
            return Ok(Redirect::found("/sign-in").into());
        };
        let (project, allowed, service) =
            match self.store(&account, &query.project, &query.service).await? {
                Ok(found) => found,
                Err(elsewhere) => return Ok(Redirect::found(elsewhere).into()),
            };
        let back = format!(
            "/projects/{}/services/{}/buckets",
            project.slug, service.slug
        );
        let Some(bucket) =
            object_stores::bucket(&self.state.database, &service.id, &query.bucket).await?
        else {
            return Ok(Redirect::found(back).into());
        };
        let keys = object_stores::keys(&self.state.database, &bucket.id).await?;
        let holder =
            crate::infra::deploy::stores::held_elsewhere(&self.state.database, &service.id).await;
        let usage = self
            .usage_of(
                &service,
                &[bucket.name.as_str().to_string()],
                holder.as_ref(),
            )
            .await
            .and_then(|mut found| found.pop().flatten());
        let store_hostname = self.store_hostname(&service).await;
        let endpoint = store_hostname.as_deref().map(|host| self.address(host));
        let public_address = bucket
            .public_hostname
            .as_deref()
            .map(|host| self.address(host));
        let derived_public = store_hostname.map(|host| format!("{}.{host}", bucket.name));

        let path = format!("{back}/{}", bucket.name);
        layout::head(bucket.name.as_str());
        let frame = Frame::load(&self.state.database, &account, Section::Projects, path).await;
        let page = crate::features::buckets::bucket_page::BucketView {
            project: &project,
            service: &service,
            bucket: &bucket,
            keys: &keys,
            usage: usage.as_ref(),
            runs_on: holder.as_ref().map(|holder| holder.name.as_str()),
            endpoint: endpoint.as_deref(),
            public_address: public_address.as_deref(),
            derived_public: derived_public.as_deref(),
            may_deploy: allowed.may_deploy(),
            error: query.error.as_deref(),
        };
        let body = crate::shell::texts::language::scoped(account.language, || {
            crate::features::buckets::bucket_page::render(&page)
                .render()
                .into_inner()
        });
        Ok(frame.render(body).into_view().into())
    }
}
