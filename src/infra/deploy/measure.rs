//! Taking every copy's reading, once a minute, and keeping what it closed.

use crate::domain::entities::service::DesiredState;
use crate::infra::metrics::{reading, Observation, Sampler};
use crate::infra::repositories::{
    copy_metrics, projects, replicas, service_events, services, volumes,
};
use crate::infra::runtime::client::Containerd;
use crate::infra::runtime::containers;

use super::Deployer;

const KEPT_MS: i64 = 90 * 86_400_000;
const EVENTS_KEPT_MS: i64 = 120 * 86_400_000;

impl Deployer {
    /// Every copy here that is meant to be running is a sample, whether it
    /// answered or not: the page tells a copy that was down from a node that
    /// was not measuring by exactly that.
    pub async fn measure(&self, sampler: &mut Sampler) {
        let now = crate::infra::clock::now_ms();
        let (Ok(services), Ok(projects), Ok(mine)) = (
            services::all(&self.database, None).await,
            projects::all(&self.database).await,
            replicas::here(&self.database).await,
        ) else {
            return;
        };
        let Ok(client) = Containerd::connect().await else {
            return;
        };
        let mut observations = Vec::new();
        let mut limits = std::collections::BTreeMap::new();
        for replica in mine.iter().filter(|replica| !replica.evicted()) {
            let Some(service) = services.iter().find(|s| s.id == replica.service_id) else {
                continue;
            };
            let Some(project) = projects.iter().find(|p| p.id == service.project_id) else {
                continue;
            };
            if service.desired_state != DesiredState::Running || service.awaiting_push() {
                continue;
            }
            let container = replica.container_id(&project.slug, &service.slug);
            let reading = match containers::status(&client, &container).await {
                Ok(Some(status)) if status.running() => super::cgroup_of(status.pid)
                    .and_then(|cgroup| reading::read(status.pid, &cgroup)),
                _ => None,
            };
            limits.insert(container.clone(), service.disk_limit);
            observations.push(Observation { container, reading });
        }

        let mut closed = sampler.observe(now, observations);
        if closed.is_empty() {
            return;
        }
        for row in &mut closed {
            row.disk = volumes::used_by(&self.config.node.data_dir, &row.container);
            row.disk_limit = row.disk.and(limits.get(&row.container).copied().flatten());
        }
        if let Err(error) = copy_metrics::write(&self.database, closed).await {
            tracing::warn!(%error, "could not keep what the copies measured");
        }
        if now % 3_600_000 < 600_000 {
            let _ = copy_metrics::forget_before(&self.database, now - KEPT_MS).await;
            let _ = service_events::forget_before(&self.database, now - EVENTS_KEPT_MS).await;
        }
    }
}
