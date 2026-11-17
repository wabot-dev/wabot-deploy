//! Tell the node holding a store of buckets what it holds.
//!
//! A store elsewhere is the database case with less in it: one copy, on the
//! node chosen when it was made, and the whole of it in every instruction.
//! Its buckets and keys are written here, where the console that edits them
//! is, and travel to the node running Garage, which applies them.

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::{DesiredState, Service};
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::infra::network::errand::{self, Kind};
use crate::infra::repositories::{object_stores, projects, replicas, services};

pub fn subject(service_id: &str) -> String {
    format!("store:{service_id}")
}

/// The node, other than this one, running a store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub id: String,
    /// Its id when this node has no row for it, which is still a node.
    pub name: String,
}

/// Which node, other than this one, runs the store. `None` is here.
pub async fn held_elsewhere(database: &SqliteDatabase, service_id: &str) -> Option<Holder> {
    let id = replicas::of_service(database, service_id)
        .await
        .ok()?
        .into_iter()
        .filter(|replica| !replica.evicted())
        .find_map(|replica| replica.node_id)?;
    let name = crate::infra::network::find(database, &id)
        .await
        .ok()
        .flatten()
        .map_or_else(|| id.clone(), |node| node.name);
    Some(Holder { id, name })
}

/// Where a change to a store's rows went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    Taken,
    NotRunning,
    /// To the node running it, which applies it when it next collects.
    Sent(String),
}

/// Hand a change to the store, wherever it runs: into the Garage here, or
/// as an errand to the node running it.
pub async fn apply(database: &SqliteDatabase, service: &Service) -> Result<Applied, String> {
    if let Some(holder) = held_elsewhere(database, &service.id).await {
        dispatch(database, service, &Default::default())
            .await
            .map_err(|error| error.to_string())?;
        return Ok(Applied::Sent(holder.name));
    }
    match crate::infra::garage::converge_if_running(database, &service.id).await? {
        true => Ok(Applied::Taken),
        false => Ok(Applied::NotRunning),
    }
}

/// Tell every node holding this store what it holds, and every node in
/// `also_tell` that it holds nothing.
///
/// **Recomputed, not emitted**, as a database's is: called after anything
/// that could have changed the store, and `queue_if_changed` makes calling
/// it often free.
pub async fn dispatch(
    database: &SqliteDatabase,
    service: &Service,
    also_tell: &std::collections::BTreeSet<String>,
) -> PlatformResult<usize> {
    if !service.kind.is_store() || !service.is_ours() {
        return Ok(0);
    }
    let Some(store) = object_stores::of_service(database, &service.id).await? else {
        return Ok(0);
    };
    let project = projects::find(database, &service.project_id)
        .await?
        .ok_or_else(|| PlatformError::Refused("no project for this store".into()))?;
    let service = services::find(database, &service.id)
        .await?
        .unwrap_or_else(|| service.clone());
    let placements = replicas::of_service(database, &service.id).await?;

    let mut by_node: std::collections::BTreeMap<String, Vec<u32>> = also_tell
        .iter()
        .map(|node| (node.clone(), Vec::new()))
        .collect();
    for replica in placements.iter().filter(|replica| !replica.evicted()) {
        if let Some(node_id) = &replica.node_id {
            by_node
                .entry(node_id.clone())
                .or_default()
                .push(replica.slot);
        }
    }
    if by_node.is_empty() {
        return Ok(0);
    }

    let buckets = object_stores::buckets(database, &service.id).await?;
    let keys = object_stores::keys_of_store(database, &service.id).await?;
    let contents = Contents::of(&buckets, &keys);
    let allowed: Vec<String> =
        crate::infra::network::candidates_for(database, service.kind.placement_capability())
            .await
            .map_err(|error| PlatformError::Refused(error.to_string()))?
            .into_iter()
            .map(|node| node.id)
            .collect();

    let mut sent = 0;
    for (node_id, slots) in by_node {
        if !slots.is_empty() && !allowed.contains(&node_id) {
            for replica in placements
                .iter()
                .filter(|replica| replica.node_id.as_deref() == Some(&*node_id))
            {
                let _ = replicas::set_last_error(
                    database,
                    &replica.id,
                    Some("that node has not agreed to keep data for this one"),
                )
                .await;
            }
            continue;
        }
        let payload = instruction(&project.name, &service, &store, slots, &contents)?;
        let queued = errand::queue_if_changed(
            database,
            &node_id,
            Kind::Store,
            &subject(&service.id),
            &payload,
        )
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;
        if queued.is_some() {
            tracing::info!(service = %service.slug, node = %node_id, "told a node what store it keeps");
            sent += 1;
        }
    }
    Ok(sent)
}

/// Take the store off every node holding it, on the way to deleting it.
///
/// Queued outright rather than when it differs, as a database's withdrawal
/// is: this runs once, and the risk worth avoiding is the message not being
/// sent at all.
pub async fn withdraw(database: &SqliteDatabase, service: &Service) -> PlatformResult<usize> {
    if !service.kind.is_store() || !service.is_ours() {
        return Ok(0);
    }
    let Some(store) = object_stores::of_service(database, &service.id).await? else {
        return Ok(0);
    };
    let project = projects::find(database, &service.project_id)
        .await?
        .ok_or_else(|| PlatformError::Refused("no project for this store".into()))?;
    let payload = instruction(
        &project.name,
        service,
        &store,
        Vec::new(),
        &Contents::default(),
    )?;
    let mut sent = 0;
    for node_id in replicas::of_service(database, &service.id)
        .await?
        .into_iter()
        .filter(|replica| !replica.evicted())
        .filter_map(|replica| replica.node_id)
        .collect::<std::collections::BTreeSet<_>>()
    {
        errand::queue(database, &node_id, Kind::Store, &payload)
            .await
            .map_err(|error| PlatformError::Refused(error.to_string()))?;
        tracing::info!(service = %service.slug, node = %node_id, "asked a node to let a store go");
        sent += 1;
    }
    Ok(sent)
}

#[derive(Default)]
struct Contents {
    buckets: Vec<errand::StoreBucket>,
    keys: Vec<errand::StoreKey>,
}

impl Contents {
    fn of(
        buckets: &[crate::domain::entities::bucket::Bucket],
        keys: &[crate::domain::entities::bucket_key::BucketKey],
    ) -> Self {
        Self {
            buckets: buckets
                .iter()
                .map(|bucket| errand::StoreBucket {
                    name: bucket.name.as_str().to_string(),
                    max_size: bucket.max_size,
                    public_hostname: bucket.public_hostname.clone(),
                    cors_origins: bucket
                        .cors_origins
                        .iter()
                        .map(|origin| origin.as_str().to_string())
                        .collect(),
                    expire_days: bucket.expire_days,
                    abort_uploads_days: bucket.abort_uploads_days,
                })
                .collect(),
            keys: keys
                .iter()
                .filter_map(|key| {
                    let bucket = buckets.iter().find(|bucket| bucket.id == key.bucket_id)?;
                    Some(errand::StoreKey {
                        bucket: bucket.name.as_str().to_string(),
                        name: key.name.clone(),
                        access_key_id: key.access_key_id.clone(),
                        secret_key: key.secret_key.clone(),
                        access: key.access.as_str().to_string(),
                    })
                })
                .collect(),
        }
    }
}

fn instruction(
    project: &str,
    service: &Service,
    store: &crate::domain::entities::object_store::ObjectStore,
    slots: Vec<u32>,
    contents: &Contents,
) -> PlatformResult<serde_json::Value> {
    serde_json::to_value(errand::Store {
        project: project.to_string(),
        service: service.name.clone(),
        image: service.image.clone(),
        version: store.version.clone(),
        memory_limit: service.memory_limit,
        disk_limit: service.disk_limit,
        rpc_secret: store.rpc_secret.clone(),
        admin_token: store.admin_token.clone(),
        slots,
        running: service.desired_state == DesiredState::Running,
        buckets: contents.buckets.clone(),
        keys: contents.keys.clone(),
    })
    .map_err(|error| PlatformError::Refused(error.to_string()))
}
