//! The images of a service this node no longer keeps.
//!
//! Every push and every deployment left an image behind, and nothing took
//! one away: each push of `nubo-testing` brought 917 MB of layers nothing
//! else had, to a node-0 with 3 GB of disk left (2026-11-17). A node keeps the image each service runs, the
//! newest, and the most recent others up to the number in its settings.
//!
//! Only a repository a plain container runs from is collected. One nothing
//! here runs is somebody's base image, pushed to be built on; a managed
//! engine's image is the node's to choose.

use std::collections::{HashMap, HashSet};

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::{Kind, Service};
use crate::domain::policies::images::{self, Held};
use crate::domain::values::image_reference::ImageReference;
use crate::infra::repositories::{releases, replicas, services};
use crate::infra::runtime::client::{ClientError, Containerd};
use crate::infra::runtime::{containers, images as runtime};

fn repository_of(image: &str) -> Option<String> {
    ImageReference::parse(image)
        .ok()
        .and_then(|reference| reference.repository())
}

/// Every repository a plain container here runs from, and then the
/// collector, so the disk comes back now rather than whenever it runs.
///
/// The collector is asked even when nothing was forgotten here: what a push
/// holds is held by its lease for a day, and nothing else wakes the
/// collector the hour that lease runs out.
pub async fn sweep(database: &SqliteDatabase) {
    let Ok(all) = services::all(database, None).await else {
        return;
    };
    let repositories: HashSet<String> = all
        .iter()
        .filter(|service| service.kind == Kind::Container)
        .filter_map(|service| repository_of(&service.image))
        .collect();
    for repository in repositories {
        collect(database, &repository).await;
    }
    let Ok(client) = Containerd::connect().await else {
        return;
    };
    let forgotten = forget_trees(database, &client).await;
    if forgotten > 0 {
        tracing::info!(forgotten, "gave back the trees of images no copy here runs");
    }
    if let Err(error) = crate::infra::runtime::leases::collect(&client).await {
        tracing::warn!(%error, "could not have containerd collect");
    }
}

/// The owner keeps the blobs of a service whose copies run elsewhere, because
/// they pull from it; the tree unpacked from them is only disk here.
async fn forget_trees(database: &SqliteDatabase, client: &Containerd) -> usize {
    let Some(_quiet) = runtime::no_container_being_made() else {
        return 0;
    };
    let (Ok(all), Ok(here)) = (
        services::all(database, None).await,
        replicas::here(database).await,
    ) else {
        return 0;
    };
    let (Ok(made_from), Ok(listed)) =
        (containers::images(client).await, runtime::all(client).await)
    else {
        return 0;
    };

    let copies_here: HashSet<&str> = here
        .iter()
        .map(|replica| replica.service_id.as_str())
        .collect();
    let mut used_here: HashSet<String> = all
        .iter()
        .filter(|service| copies_here.contains(service.id.as_str()))
        .filter_map(|service| repository_of(&service.image))
        .collect();
    used_here.extend(made_from.iter().filter_map(|image| repository_of(image)));
    let containers_run_from: HashSet<String> = all
        .iter()
        .filter(|service| service.kind == Kind::Container)
        .filter_map(|service| repository_of(&service.image))
        .collect();

    let mut records = Vec::new();
    for record in &listed {
        // An image whose config cannot be read here was never unpacked here.
        if let Ok(config) = runtime::config_digest(client, &record.reference).await {
            records.push((repository_of(&record.reference), config));
        }
    }

    let mut forgotten = 0;
    for config in images::trees_nobody_here_needs(&records, &containers_run_from, &used_here) {
        match runtime::forget_tree(client, config).await {
            Ok(true) => forgotten += 1,
            Ok(false) => {}
            Err(error) => tracing::warn!(%config, %error, "could not give back a tree"),
        }
    }
    forgotten
}

/// The repository of `image`, collected.
pub async fn collect_image(database: &SqliteDatabase, image: &str) {
    if let Some(repository) = repository_of(image) {
        collect(database, &repository).await;
    }
}

/// One at a time: a push that deploys asks twice within a millisecond, once
/// for the deployment and once for the tag, and two collections reading the
/// same list each tried to forget what the other already had.
static COLLECTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn collect(database: &SqliteDatabase, repository: &str) {
    let _alone = COLLECTING.lock().await;
    let Ok(all) = services::all(database, None).await else {
        return;
    };
    let using: Vec<Service> = all
        .into_iter()
        .filter(|service| repository_of(&service.image).as_deref() == Some(repository))
        .collect();
    if !using.iter().any(|service| service.kind == Kind::Container) {
        return;
    }
    let Ok(client) = Containerd::connect().await else {
        return;
    };
    let Ok(listed) = runtime::all(&client).await else {
        return;
    };

    let mut held = Vec::new();
    let mut read: HashMap<String, Vec<String>> = HashMap::new();
    for record in listed
        .into_iter()
        .filter(|record| repository_of(&record.reference).as_deref() == Some(repository))
    {
        if !read.contains_key(&record.digest) {
            let children = runtime::listed(&client, &record.digest, record.size).await;
            read.insert(record.digest.clone(), children);
        }
        held.push(Held {
            children: read[&record.digest].clone(),
            reference: record.reference,
            digest: record.digest,
            updated_at: record.updated_at,
        });
    }

    let resolve = |image: &str| -> Option<String> {
        match ImageReference::parse(image).ok()?.digest() {
            Some(digest) => Some(digest.to_string()),
            None => held
                .iter()
                .find(|record| record.reference == image)
                .map(|record| record.digest.clone()),
        }
    };

    let mut pushed: HashMap<String, i64> = HashMap::new();
    let mut in_use: HashSet<String> = HashSet::new();
    for service in &using {
        let history = releases::of_service(database, &service.id)
            .await
            .unwrap_or_default();
        for release in &history {
            let known = pushed.entry(release.digest.clone()).or_insert(0);
            *known = (*known).max(release.created_at);
        }
        // Whatever tag it arrived under. The node owning a service it runs
        // nowhere itself has no container to read, and a copy elsewhere pulls
        // this digest from here every time it restarts.
        in_use.extend(
            history
                .iter()
                .filter(|release| release.deployed_at.is_some())
                .map(|release| release.digest.clone()),
        );
        in_use.extend(resolve(&service.image));
    }
    let Ok(made_from) = containers::images(&client).await else {
        return;
    };
    for image in made_from
        .iter()
        .filter(|image| repository_of(image).as_deref() == Some(repository))
    {
        in_use.extend(resolve(image));
    }

    let keep = crate::infra::node::settings::images_kept(database)
        .await
        .unwrap_or(images::KEPT_BY_DEFAULT);
    let surplus = images::surplus(&held, &pushed, &in_use, keep);
    if surplus.forget.is_empty() {
        return;
    }

    let mut forgotten = 0;
    for reference in &surplus.forget {
        match runtime::forget(&client, reference).await {
            Ok(()) => forgotten += 1,
            Err(ClientError::Call { source, .. }) if source.code() == tonic::Code::NotFound => {}
            Err(error) => tracing::warn!(image = %reference, %error, "could not forget an image"),
        }
    }
    for digest in &surplus.dropped {
        for service in &using {
            if let Err(error) = releases::forget_digest(database, &service.id, digest).await {
                tracing::warn!(service = %service.slug, %digest, %error, "could not forget its releases");
            }
        }
        if let Err(error) =
            crate::infra::registry::manifests::forget_digest(database, repository, digest).await
        {
            tracing::warn!(%repository, %digest, %error, "could not forget its tags");
        }
    }
    tracing::info!(
        %repository,
        keep,
        images = surplus.dropped.len(),
        records = forgotten,
        "collected images"
    );
}
