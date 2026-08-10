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

use crate::platform::{projects, releases, replicas, services};

/// Bring one service's container in line with its rows.
#[command("deploy-service")]
#[derive(Debug, Serialize, Deserialize, Validate)]
pub struct DeployService {
    #[is_not_empty]
    pub service_id: String,
    /// A release to pin, for deploying an older image — which is what
    /// a rollback is. `None` deploys whatever the service names.
    pub release_id: Option<String>,
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

        // Every copy of it lives on another node. Nothing to deploy and
        // nothing wrong — the same shape as a service deleted while the
        // job waited, and for the same reason: retrying cannot make a
        // replica appear here, so a job that fails on it fails for ever
        // and fills the journal with an ERROR for a node that is doing
        // exactly what it was told.
        if replicas::here_for(&self.database, &service.id)
            .await
            .map_err(|error| AsyncError::Handler(error.to_string()))?
            .is_none()
        {
            // The routes still have to be recomputed, and this is
            // exactly the moment they are wrong: a service whose last
            // copy just left this node has a route pointing at a
            // container that is gone. Returning without it left the
            // name proxying to an address nothing answers on — which is
            // worse than the ERROR this replaced, because it fails
            // silently.
            self.deployer.sync_routes().await;
            tracing::info!(
                service = %service.slug,
                "every copy of this service runs on another node; nothing to deploy here"
            );
            return Ok(());
        }

        match &data.release_id {
            None => {
                self.deployer
                    .deploy(&project, &service)
                    .await
                    .map(|_| ())
                    // The reason is already on the service row —
                    // `deploy` puts it there — so this only decides
                    // whether the job retries.
                    .map_err(|error| AsyncError::Handler(error.to_string()))
            }
            Some(id) => {
                let release = releases::find(&self.database, id)
                    .await
                    .map_err(|error| AsyncError::Handler(error.to_string()))?;
                let Some(release) = release.filter(|found| found.service_id == service.id) else {
                    tracing::warn!(release = %id, "no such release for this service");
                    return Ok(());
                };
                self.deployer.deploy_release(&service, &release).await;
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
