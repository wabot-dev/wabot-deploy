//! Deploying as a job, not as a request.
//!
//! ## Why a request was the wrong place
//!
//! `deploy` pulls an image, unpacks it, creates a container, joins a
//! network and rewrites the route table. That ran inside the POST, so
//! the browser held an open request for the whole of it — and somebody
//! who got impatient and reloaded was offered "confirm form
//! resubmission", because the 303 that would have ended it had not
//! been sent yet. The redirect-after-post was correct all along; the
//! POST was simply not finished.
//!
//! ## Why the framework's runner and not a spawned task
//!
//! A spawn loses the work when the process is replaced, and the
//! process being replaced is a *feature* here — the updater ends by
//! restarting the node. A deployment that vanished mid-way would leave
//! a container half-created with nothing recording that it was. The
//! job survives in SQLite, and the runner picks it up on the next
//! tick.
//!
//! It also brings retries and the drain phase, both of which would
//! otherwise be written here and be a second, worse copy.
//!
//! ## Two registrations, both easy to miss
//!
//! The handler resolves `SqliteDatabase` from the container, and
//! nothing else puts it there — `api::register` takes the database and
//! keeps it inside `NodeStatus`. Without it the worker panics building
//! the handler while the button still answers happily, which is how
//! the first deployment on a real node did nothing at all.
//!
//! And `JobExecutor` keeps its own set of command names, separate from
//! `CommandRegistry`. `run_command`'s immediate spawn checks *that*
//! one, so a command registered only with the registry is stored and
//! never run. `run_async_workers` does both; anything wiring this by
//! hand has to as well.
//!
//! ## Ids, not rows
//!
//! The payload names the service; the handler loads it. A row
//! serialized at enqueue would be the state when the button was
//! pressed, and the whole point of queueing is that some time passes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use wabot::prelude::*;

use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::infra::repositories::projects;
use crate::infra::repositories::releases;
use crate::infra::repositories::replicas;
use crate::infra::repositories::services;

/// Bring one service's container in line with its rows.
#[command("deploy-service")]
#[derive(Debug, Serialize, Deserialize, Validate)]
pub struct DeployService {
    #[is_not_empty]
    pub service_id: String,
    /// A release to pin, for deploying an older image — which is what
    /// a rollback is. `None` deploys whatever the service names.
    pub release_id: Option<String>,
    /// When it was asked for, so that a stop made after it stands. Without
    /// it the retry of a deployment that had failed, or one queued behind
    /// another, started back a service stopped while it waited. `None` in a
    /// job queued by a version without it.
    #[serde(default)]
    pub asked_at: Option<i64>,
}

impl DeployService {
    pub fn new(service_id: &str, release_id: Option<&str>) -> Self {
        Self {
            service_id: service_id.to_string(),
            release_id: release_id.map(str::to_string),
            asked_at: Some(crate::infra::clock::distinct_now_ms()),
        }
    }
}

/// Two retries, at 15 seconds and a minute.
///
/// The list is four long for two retries because the delay index
/// advances twice per attempt — `set_as_started` increments it and
/// `set_as_failed` increments it again — so only the odd positions are
/// ever read. That is wabot-ts's behaviour, ported deliberately; the
/// pairs are how you write it without pretending otherwise.
///
/// Retrying at all is for the transient case: containerd not answering
/// for a moment, a registry that timed out. An image that does not
/// exist fails all three times and leaves the reason on the service
/// row, which is where somebody will look.
///
/// The attribute goes on the struct and `handle` lives in a plain
/// inherent impl — `expand_command_handler` parses an `ItemStruct`.
/// The framework's own skill shows it on the impl, which does not
/// compile.
#[injectable]
#[command_handler(DeployService, retry_delays = [15, 15, 60, 60])]
pub struct DeployHandler {
    deployer: Arc<super::Deployer>,
    database: Arc<wabot::sqlite::SqliteDatabase>,
    /// For the report this sends when a deployment finishes: reading what
    /// each container is using needs the deployer, and it is resolved out
    /// of here rather than held twice.
    container: Container,
}

impl DeployHandler {
    async fn handle(&self, data: DeployService) -> Result<(), AsyncError> {
        // By id, out of the whole list: `services` has no by-id
        // lookup, and a node holds a handful of them.
        let Some(service) = services::all(&self.database, None)
            .await
            .map_err(|error| AsyncError::Handler(error.to_string()))?
            .into_iter()
            .find(|found| found.id == data.service_id)
        else {
            // Deleted while the job waited. Nothing to deploy and
            // nothing wrong — failing here would retry twice over a
            // service that is gone.
            tracing::info!(service = %data.service_id, "no such service; nothing to deploy");
            return Ok(());
        };

        let Some(project) = projects::find(&self.database, &service.project_id)
            .await
            .map_err(|error| AsyncError::Handler(error.to_string()))?
        else {
            tracing::warn!(service = %service.slug, "no project for this service");
            return Ok(());
        };

        // Every copy of it on another node is still `deploy`'s to handle:
        // it tells those nodes to run it. This returned before reaching it,
        // so Deploy on the owner did nothing for a service placed entirely
        // elsewhere (Jorge, `nubo-testing` on node-0, 2026-11-14).
        let nothing_here = replicas::here_for(&self.database, &service.id)
            .await
            .map_err(|error| AsyncError::Handler(error.to_string()))?
            .is_none();
        let outcome = self.carry_out(&project, &service, &data).await;
        // A service whose last copy just left this node has a route
        // pointing at a container that is gone.
        if nothing_here {
            self.deployer.sync_routes().await;
        }
        outcome
    }

    async fn carry_out(
        &self,
        project: &Project,
        service: &Service,
        data: &DeployService,
    ) -> Result<(), AsyncError> {
        match &data.release_id {
            None => {
                let outcome = self
                    .deployer
                    .deploy(project, service, data.asked_at)
                    .await
                    .map(|_| ())
                    // The reason is already on the service row —
                    // `deploy` puts it there — so this only decides
                    // whether the job retries.
                    .map_err(|error| AsyncError::Handler(error.to_string()));
                if outcome.is_ok() && service.origin_node_id.is_none() {
                    use crate::domain::entities::service_event::EventKind;
                    use crate::infra::repositories::service_events::{self, Happened};
                    let _ = service_events::record(
                        &self.database,
                        Happened {
                            detail: Some(&service.image),
                            ..Happened::now(&service.id, EventKind::Deployed)
                        },
                    )
                    .await;
                }

                // And whoever asked for this hears now.
                //
                // A deployment that arrived on an errand was settled
                // before it ran — it is queued as this node's own job,
                // which is what "obeying is local" means — so the
                // authority's next scheduled report is up to fifteen
                // seconds away, and until it lands the console that
                // pressed the button says "waiting for that node" about
                // a container that has been up the whole time. Reported
                // whether it worked or not: a failure is the answer
                // somebody is waiting for too.
                //
                // A no-op on a node that takes instructions from nobody,
                // which is every single-node install.
                crate::infra::network::collect::report_now(
                    &self.database,
                    self.deployer.config(),
                    &self.container,
                )
                .await;
                outcome
            }
            Some(id) => {
                let release = releases::find(&self.database, id)
                    .await
                    .map_err(|error| AsyncError::Handler(error.to_string()))?;
                let Some(release) = release.filter(|found| found.service_id == service.id) else {
                    tracing::warn!(release = %id, "no such release for this service");
                    return Ok(());
                };
                self.deployer
                    .deploy_release(service, &release, data.asked_at)
                    .await;
                Ok(())
            }
        }
    }
}

/// Which services have a deployment running right now.
///
/// Asked of the job store rather than kept in memory: a queue that
/// survives a restart needs an answer that survives one too, and after
/// an update the console would otherwise report nothing in flight
/// while the worker was mid-pull.
///
/// Running, not merely queued. `run_command` spawns immediately, so
/// the gap between the two is a moment — and a page that showed
/// "deploying" for a job the executor had not picked up would be
/// promising something it could not see.
pub async fn deploying(container: &Container) -> std::collections::BTreeSet<String> {
    let Ok(repository) = container.try_resolve::<wabot::async_jobs::DynJobRepository>() else {
        return Default::default();
    };
    let Ok(running) = repository.0.find_running().await else {
        // A store that cannot answer is not a reason to fail a page:
        // the worst case is a badge that does not appear.
        return Default::default();
    };

    running
        .iter()
        .filter(|job| wabot::async_jobs::job::command_name(job) == DeployService::COMMAND_NAME)
        .filter_map(|job| {
            wabot::async_jobs::job::command_data(job)
                .get("service_id")?
                .as_str()
                .map(str::to_string)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::service::DesiredState;

    /// Deploy on the owner of a service placed entirely on another node did
    /// nothing: the job returned before `deploy`, which is what tells that
    /// node to run it (Jorge, `nubo-testing` on node-0, 2026-11-14).
    #[tokio::test]
    async fn deploying_a_service_placed_entirely_elsewhere_tells_that_node_to_run_it() {
        let (database, _, service) = super::super::tests::placed_elsewhere().await;
        let database = Arc::new(database);
        services::set_desired_state(&database, &service.id, DesiredState::Stopped)
            .await
            .expect("stopped");
        let handler = DeployHandler {
            deployer: Arc::new(super::super::Deployer::new(
                database.clone(),
                &crate::infra::config::Config::default(),
            )),
            database: database.clone(),
            container: Container::new(),
        };

        handler
            .handle(DeployService::new(&service.id, None))
            .await
            .expect("deployed");

        let told = crate::infra::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands")
            .pop()
            .expect("the far node was told nothing");
        assert_eq!(told.payload["running"], serde_json::json!(true));
        assert_eq!(
            services::find(&database, &service.id)
                .await
                .expect("query")
                .expect("there")
                .desired_state,
            DesiredState::Running
        );
    }

    /// A deployment that failed is tried again 15 and 60 seconds later,
    /// and the retry set `running` like any deployment: a service stopped
    /// in between was started again by a job asked for before the stop.
    #[tokio::test]
    async fn the_retry_of_a_deployment_asked_for_before_a_stop_does_not_start_it_again() {
        let (database, project, service) = super::super::tests::placed_elsewhere().await;
        let database = Arc::new(database);
        let deployer = Arc::new(super::super::Deployer::new(
            database.clone(),
            &crate::infra::config::Config::default(),
        ));
        let handler = DeployHandler {
            deployer: deployer.clone(),
            database: database.clone(),
            container: Container::new(),
        };
        let asked = DeployService::new(&service.id, None);

        // Its local half needs a containerd that is not here; the intent
        // and the errands are written before it.
        let _ = deployer.stop(&project, &service).await;
        handler.handle(asked).await.expect("retried");

        assert_eq!(
            services::find(&database, &service.id)
                .await
                .expect("query")
                .expect("there")
                .desired_state,
            DesiredState::Stopped
        );
        let told = crate::infra::network::errand::waiting(&database, "nd-far")
            .await
            .expect("errands");
        assert!(
            told.iter()
                .all(|errand| errand.payload["running"] == serde_json::json!(false)),
            "the far node was told to run it again: {:?}",
            told.iter()
                .map(|errand| &errand.payload)
                .collect::<Vec<_>>()
        );
    }
}
