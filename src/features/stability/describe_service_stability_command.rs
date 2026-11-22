use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::copy_metric::{CopyMetric, BUCKET_MS};
use crate::domain::entities::project::Project;
use crate::domain::entities::replica::Replica;
use crate::domain::entities::service::{Kind, Service};
use crate::domain::entities::service_event::{EventKind, ServiceEvent};
use crate::domain::policies::stability::{self, day_of, Copy, Input, Row, DAY_MS};
use crate::features::stability::service_stability_response::{
    Chart, CopyKey, EventLine, ServiceStabilityResponse,
};
use crate::infra::repositories::{config_history, copy_metrics, replicas, service_events, volumes};

/// What a service's stability page shows, whoever is looking.
pub struct DescribeServiceStability<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub days: u32,
    pub day: Option<i64>,
    pub event: Option<&'a str>,
}

struct Held {
    replica: Replica,
    container: String,
    node: Option<String>,
    metrics: Vec<CopyMetric>,
}

impl DescribeServiceStability<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<ServiceStabilityResponse, String> {
        let (project, service) = (self.project, self.service);
        let unreadable = |error: String| {
            tracing::warn!(%error, service = %service.slug, "could not read a service's history");
            "its history could not be read".to_string()
        };
        let now = crate::infra::clock::now_ms();
        let today = day_of(now);
        let first = today - i64::from(self.days - 1) * DAY_MS;
        let from = first - 7 * DAY_MS;

        let mut copies: Vec<Replica> = replicas::of_service(database, &service.id)
            .await
            .map_err(|error| unreadable(error.to_string()))?
            .into_iter()
            .filter(|replica| !replica.evicted())
            .collect();
        copies.sort_by_key(|replica| replica.slot);

        let mut unreachable = Vec::new();
        let mut held = Vec::new();
        for replica in copies {
            let container = replica.container_id(&project.slug, &service.slug);
            let node = match (&replica.node_id, replica.is_here()) {
                (Some(node_id), false) => {
                    let name = crate::infra::network::find(database, node_id)
                        .await
                        .ok()
                        .flatten()
                        .map(|node| node.name)
                        .unwrap_or_else(|| node_id.clone());
                    if service.origin_node_id.is_none() {
                        let gathered = crate::infra::network::gather_history(
                            database,
                            node_id,
                            (&project.name, &service.name, replica.slot),
                            &container,
                            &service.id,
                            (from, now),
                        )
                        .await;
                        if let Err(reason) = gathered {
                            unreachable.push((name.clone(), reason));
                        }
                    }
                    Some(name)
                }
                _ => None,
            };
            let metrics = copy_metrics::of_container(database, &container, from, now)
                .await
                .map_err(|error| unreadable(error.to_string()))?;
            held.push(Held {
                replica,
                container,
                node,
                metrics,
            });
        }

        let mut events = service_events::of_service(database, &service.id, now)
            .await
            .map_err(|error| unreadable(error.to_string()))?;
        events.extend(env_changes(database, service).await);
        events.sort_by_key(|event| event.at);

        let with_data = service.kind.is_database()
            || service.kind.is_store()
            || volumes::of_service(database, &service.id)
                .await
                .map(|list| !list.is_empty())
                .unwrap_or(false);
        let plan = crate::infra::backup::plan::plan(database).await;
        let backup_every = (with_data && plan.cadence.interval_days() > 0)
            .then(|| plan.cadence.interval_days() * DAY_MS);
        let mut rows = vec![Row::Availability, Row::Memory, Row::Cpu, Row::Network];
        let mut charts = vec![Chart::Memory, Chart::Cpu, Chart::Network];
        if with_data {
            rows.push(Row::Disk);
            charts.extend([Chart::Disk, Chart::Io]);
        }
        if backup_every.is_some() {
            rows.push(Row::Backups);
        }
        if service.kind == Kind::Postgres && held.len() > 1 {
            rows.push(Row::Replicas);
        }

        let policy_copies: Vec<Copy> = held
            .iter()
            .map(|copy| Copy {
                slot: copy.replica.slot,
                container: &copy.container,
                metrics: &copy.metrics,
                current: true,
            })
            .collect();
        let input = Input {
            now,
            copies: &policy_copies,
            events: &events,
            rows: &rows,
            backup_every,
        };
        let strip = stability::days(&input, first, self.days as usize);
        let mut everything = events.clone();
        everything.extend(stability::derived(&input, first, now));
        everything.sort_by_key(|event| event.at);

        let pinned_day = self
            .event
            .and_then(|id| everything.iter().find(|event| event.id == id))
            .map(|event| day_of(event.at));
        let measured_since = stability::since(&policy_copies).map(day_of);
        let since = measured_since.unwrap_or(today);
        let selected = self
            .day
            .or(pinned_day)
            .or_else(|| stability::worth_opening(&strip))
            .unwrap_or(today)
            .clamp(first.max(since).min(today), today);

        let buckets = match selected == today {
            true => (((now - today) / BUCKET_MS) as usize).max(1),
            false => (DAY_MS / BUCKET_MS) as usize,
        };
        let series = held
            .iter()
            .map(|copy| {
                let mut line = vec![None; buckets];
                for metric in &copy.metrics {
                    let index = (metric.bucket - selected) / BUCKET_MS;
                    if (0..buckets as i64).contains(&index) {
                        line[index as usize] = Some(metric.clone());
                    }
                }
                line
            })
            .collect();

        let slot_of = |container: Option<&str>| {
            container.and_then(|container| held.iter().find(|copy| copy.container == container))
        };
        let line = |event: &ServiceEvent| {
            let copy = slot_of(event.container.as_deref());
            EventLine {
                id: event.id.clone(),
                kind: event.kind,
                severity: event.severity(),
                at: event.at,
                until: event.until,
                slot: copy.map(|copy| copy.replica.slot),
                node: copy.and_then(|copy| copy.node.clone()),
                code: event.code,
                detail: event.detail.clone(),
            }
        };
        let day_events = everything
            .iter()
            .filter(|event| event.at < selected + DAY_MS && event.ends() >= selected)
            .map(line)
            .collect();
        let period_events = everything
            .iter()
            .rev()
            .filter(|event| event.ends() >= first && !event.kind.routine())
            .map(line)
            .collect();
        let deploys = strip
            .iter()
            .filter_map(|day| {
                events
                    .iter()
                    .rfind(|event| {
                        event.kind == EventKind::Deployed && day_of(event.at) == day.start
                    })
                    .map(|event| {
                        (
                            day.start,
                            crate::features::stability::texts::tag(
                                event.detail.as_deref().unwrap_or(""),
                            ),
                        )
                    })
            })
            .collect();

        Ok(ServiceStabilityResponse {
            days: self.days,
            today,
            selected,
            since: measured_since,
            rows,
            strip,
            deploys,
            copies: held
                .iter()
                .map(|copy| CopyKey {
                    slot: copy.replica.slot,
                    node: copy.node.clone(),
                })
                .collect(),
            buckets,
            series,
            charts,
            day_events,
            period_events,
            pinned: self
                .event
                .filter(|id| everything.iter().any(|event| event.id == *id))
                .map(str::to_string),
            unreachable,
        })
    }
}

/// Each saved change of the variables after the first, with the names that
/// changed and never their values.
async fn env_changes(database: &SqliteDatabase, service: &Service) -> Vec<ServiceEvent> {
    let Ok(mut revisions) = config_history::of_service(database, &service.id).await else {
        return Vec::new();
    };
    revisions.sort_by_key(|revision| revision.created_at);
    revisions
        .windows(2)
        .map(|pair| {
            let (before, after) = (&pair[0].env, &pair[1].env);
            let mut names: Vec<&str> = before
                .keys()
                .chain(after.keys())
                .filter(|name| before.get(*name) != after.get(*name))
                .map(String::as_str)
                .collect();
            names.sort_unstable();
            names.dedup();
            ServiceEvent {
                id: pair[1].id.clone(),
                service_id: service.id.clone(),
                container: None,
                kind: EventKind::EnvChanged,
                at: pair[1].created_at,
                until: None,
                code: None,
                detail: (!names.is_empty()).then(|| names.join(", ")),
            }
        })
        .collect()
}
