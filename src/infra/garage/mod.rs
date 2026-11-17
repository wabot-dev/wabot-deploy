//! Garage's admin API, from the node to a store's container, and the pass
//! that makes Garage hold what the rows say.
//!
//! On the container's address and never through the edge: the admin port is
//! not routed, and its token is the whole of the store's authority.

use std::net::Ipv4Addr;
use std::time::Duration;

use http_body_util::{BodyExt, Full};
use hyper::body::Bytes;
use hyper::{Method, Request};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::bucket_key::BucketKey;
use crate::domain::entities::object_store::ObjectStore;
use crate::domain::policies::garage::{self, Permissions};
use crate::domain::values::bucket_name::BucketName;
use crate::domain::values::cors_origin::CorsOrigin;

const TIMEOUT: Duration = Duration::from_secs(30);

const PEER_LIST: &str = "peer_list";

/// How long a store that has just started gets to answer. Garage opens its
/// metadata and lays out its one node before the API does.
const STARTING: Duration = Duration::from_secs(60);

const NOTHING: Permissions = Permissions {
    read: false,
    write: false,
    owner: false,
};

pub struct Admin {
    base: String,
    token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedBucket {
    id: String,
    global_aliases: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedKey {
    id: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct HeldPermissions {
    #[serde(default)]
    read: bool,
    #[serde(default)]
    write: bool,
    #[serde(default)]
    owner: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HolderOfBucket {
    access_key_id: String,
    permissions: HeldPermissions,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub objects: u64,
    pub bytes: u64,
    pub unfinished_uploads: u64,
    #[serde(default)]
    keys: Vec<HolderOfBucket>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MintedKey {
    access_key_id: String,
    secret_access_key: Option<String>,
}

impl From<HeldPermissions> for Permissions {
    fn from(held: HeldPermissions) -> Self {
        Permissions {
            read: held.read,
            write: held.write,
            owner: held.owner,
        }
    }
}

impl Usage {
    /// What the node running a store elsewhere reported.
    pub fn reported(measured: &crate::domain::entities::bucket::Measured) -> Self {
        Self {
            objects: measured.objects,
            bytes: measured.bytes,
            unfinished_uploads: measured.unfinished_uploads,
            keys: Vec::new(),
        }
    }
}

/// What every bucket of the store running here holds, for the report to its
/// owner. `None` when the store does not answer within `within`: a report
/// is sent every few seconds, and a store that hangs must not hold it up.
pub async fn measure(
    database: &SqliteDatabase,
    service_id: &str,
    within: std::time::Duration,
) -> Option<Vec<crate::domain::entities::bucket::Measured>> {
    let admin = admin_for(database, service_id).await.ok()?;
    let buckets = crate::infra::repositories::object_stores::buckets(database, service_id)
        .await
        .ok()?;
    tokio::time::timeout(within, async {
        let mut measured = Vec::with_capacity(buckets.len());
        for bucket in buckets {
            let usage = admin.usage(bucket.name.as_str()).await.ok()??;
            measured.push(crate::domain::entities::bucket::Measured {
                name: bucket.name.as_str().to_string(),
                objects: usage.objects,
                bytes: usage.bytes,
                unfinished_uploads: usage.unfinished_uploads,
            });
        }
        Some(measured)
    })
    .await
    .ok()?
}

impl Admin {
    pub fn at(address: Ipv4Addr, token: &str) -> Self {
        Self {
            base: format!("http://{address}:{}", garage::ADMIN_PORT),
            token: token.to_string(),
        }
    }

    async fn call(
        &self,
        method: Method,
        operation: &str,
        body: Option<serde_json::Value>,
    ) -> Result<Vec<u8>, String> {
        let (status, bytes) = self.send(method, operation, body).await?;
        if !(200..300).contains(&status) {
            let name = operation.split('?').next().unwrap_or(operation);
            return Err(format!(
                "the store refused {name} ({status}): {}",
                said(&bytes)
            ));
        }
        Ok(bytes)
    }

    /// Gone already is what a deletion was for.
    async fn delete(&self, operation: &str) -> Result<(), String> {
        match self.send(Method::POST, operation, None).await? {
            (404, _) => Ok(()),
            (status, _) if (200..300).contains(&status) => Ok(()),
            (status, bytes) => Err(format!(
                "the store refused {} ({status}): {}",
                operation.split('?').next().unwrap_or(operation),
                said(&bytes)
            )),
        }
    }

    async fn send(
        &self,
        method: Method,
        operation: &str,
        body: Option<serde_json::Value>,
    ) -> Result<(u16, Vec<u8>), String> {
        if cfg!(test) {
            return Err("a test does not reach a store".into());
        }
        let client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();
        let body = body.map(|json| json.to_string()).unwrap_or_default();
        let request = Request::builder()
            .method(method)
            .uri(format!("{}{operation}", self.base))
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/json")
            .body(Full::new(Bytes::from(body)))
            .map_err(|error| error.to_string())?;
        let name = operation.split('?').next().unwrap_or(operation);
        let response = tokio::time::timeout(TIMEOUT, client.request(request))
            .await
            .map_err(|_| {
                format!(
                    "the store did not answer {name} within {}s",
                    TIMEOUT.as_secs()
                )
            })?
            .map_err(|error| format!("the store could not be reached for {name}: {error}"))?;
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|error| format!("reading what the store said to {name}: {error}"))?
            .to_bytes()
            .to_vec();
        Ok((status.as_u16(), bytes))
    }

    async fn read<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        operation: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T, String> {
        let bytes = self.call(method, operation, body).await?;
        serde_json::from_slice(&bytes)
            .map_err(|error| format!("the store's answer to {operation} did not read: {error}"))
    }

    /// `/health` answers without the token, so a store whose token is wrong
    /// is told apart from one that is not up yet.
    pub async fn wait_until_up(&self) -> Result<(), String> {
        let started = std::time::Instant::now();
        loop {
            match self.call(Method::GET, "/health", None).await {
                Ok(_) => return Ok(()),
                Err(error) if started.elapsed() >= STARTING => {
                    return Err(format!(
                        "the store did not come up within {}s: {error}",
                        STARTING.as_secs()
                    ))
                }
                Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
            }
        }
    }

    async fn buckets(&self) -> Result<Vec<ListedBucket>, String> {
        self.read(Method::GET, "/v2/ListBuckets", None).await
    }

    async fn create_bucket(&self, name: &str) -> Result<String, String> {
        let made: ListedBucket = self
            .read(
                Method::POST,
                "/v2/CreateBucket",
                Some(serde_json::json!({ "globalAlias": name })),
            )
            .await?;
        Ok(made.id)
    }

    async fn id_of(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self
            .buckets()
            .await?
            .into_iter()
            .find(|bucket| bucket.global_aliases.iter().any(|alias| alias == name))
            .map(|bucket| bucket.id))
    }

    pub async fn usage(&self, name: &str) -> Result<Option<Usage>, String> {
        let Some(id) = self.id_of(name).await? else {
            return Ok(None);
        };
        self.read(Method::GET, &format!("/v2/GetBucketInfo?id={id}"), None)
            .await
            .map(Some)
    }

    /// Refused by Garage while the bucket holds an object, which is the rule
    /// the console states, so the two cannot disagree.
    pub async fn delete_bucket(&self, name: &str) -> Result<(), String> {
        let Some(id) = self.id_of(name).await? else {
            return Ok(());
        };
        self.delete(&format!("/v2/DeleteBucket?id={id}")).await
    }

    async fn keys(&self) -> Result<Vec<ListedKey>, String> {
        self.read(Method::GET, "/v2/ListKeys", None).await
    }

    /// Garage chooses both halves. Its specification keeps `ImportKey` for
    /// giving a key back, and says not to use it to choose identifiers.
    pub async fn mint_key(&self, name: &str) -> Result<(String, String), String> {
        let minted: MintedKey = self
            .read(
                Method::POST,
                "/v2/CreateKey",
                Some(serde_json::json!({ "name": name, "neverExpires": true })),
            )
            .await?;
        let secret = minted
            .secret_access_key
            .ok_or("the store minted a key and did not say its secret")?;
        Ok((minted.access_key_id, secret))
    }

    /// `false` for an id Garage has deleted: it keeps the id for ever and
    /// answers 409, so that a key once revoked cannot be brought back by
    /// whoever still has its secret.
    async fn give_back(&self, key: &BucketKey) -> Result<bool, String> {
        let body = serde_json::json!({
            "accessKeyId": key.access_key_id,
            "secretAccessKey": key.secret_key,
            "name": key.name,
        });
        match self.send(Method::POST, "/v2/ImportKey", Some(body)).await? {
            (409, _) => Ok(false),
            (status, _) if (200..300).contains(&status) => Ok(true),
            (status, bytes) => Err(format!(
                "the store refused /v2/ImportKey ({status}): {}",
                said(&bytes)
            )),
        }
    }

    pub async fn delete_key(&self, access_key_id: &str) -> Result<(), String> {
        self.delete(&format!("/v2/DeleteKey?id={access_key_id}"))
            .await
    }

    async fn change(
        &self,
        operation: &str,
        bucket_id: &str,
        access_key_id: &str,
        permissions: Permissions,
    ) -> Result<(), String> {
        self.call(
            Method::POST,
            operation,
            Some(serde_json::json!({
                "bucketId": bucket_id,
                "accessKeyId": access_key_id,
                "permissions": {
                    "read": permissions.read,
                    "write": permissions.write,
                    "owner": permissions.owner,
                },
            })),
        )
        .await
        .map(|_| ())
    }

    /// Garage's website port finds a bucket by the `Host` it was asked at,
    /// so a public bucket's name is an alias of it there.
    async fn alias(&self, operation: &str, bucket_id: &str, alias: &str) -> Result<(), String> {
        self.call(
            Method::POST,
            operation,
            Some(serde_json::json!({ "bucketId": bucket_id, "globalAlias": alias })),
        )
        .await
        .map(|_| ())
    }

    pub async fn snapshot(&self) -> Result<(), String> {
        self.call(Method::POST, "/v2/CreateMetadataSnapshot?node=self", None)
            .await
            .map(|_| ())
    }
}

fn said(bytes: &[u8]) -> String {
    #[derive(Deserialize)]
    struct Said {
        message: String,
    }
    match serde_json::from_slice::<Said>(bytes) {
        Ok(said) => said.message,
        Err(_) => String::from_utf8_lossy(bytes).trim().to_string(),
    }
}

/// Put a backup's copy of a store's volume where a restored store will
/// find it, unless it is there already from a deployment that brought it
/// and then failed.
///
/// Copied beside the volume and renamed into it, so a deployment killed
/// half-way through the copy leaves no metadata in the volume, and the next
/// one starts the copy again rather than opening half of it.
///
/// The copy carries the original's own list of where its peers were, which
/// names the original's address. The original is likely still running
/// there, so the copy starts without the list.
pub fn bring_volume(copy: &std::path::Path, volume: &std::path::Path) -> std::io::Result<bool> {
    use std::io::{Error, ErrorKind};
    let absent_is_fine = |result: std::io::Result<()>| match result {
        Err(error) if error.kind() != ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    };
    if volume.join(garage::METADATA_DIR).exists() {
        return Ok(false);
    }
    if !copy.join(garage::METADATA_DIR).is_dir() {
        return Err(Error::new(
            ErrorKind::NotFound,
            "the backup's copy holds no metadata directory",
        ));
    }
    let name = volume
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "a volume with no name"))?;
    let staging = volume.with_file_name(format!(".{}.bringing", name.to_string_lossy()));
    absent_is_fine(std::fs::remove_dir_all(&staging))?;
    crate::infra::backup::copy_tree(copy, &staging)?;
    absent_is_fine(std::fs::remove_file(
        staging.join(garage::METADATA_DIR).join(PEER_LIST),
    ))?;
    absent_is_fine(std::fs::remove_dir_all(volume))?;
    std::fs::rename(&staging, volume)?;
    Ok(true)
}

/// Make a copy of a store's volume open as the snapshot Garage just wrote:
/// the newest one becomes the metadata, the live file and its journal go,
/// and so do the snapshots, which the copy would otherwise carry twice.
pub fn carry_newest_snapshot(volume: &std::path::Path) -> std::io::Result<()> {
    let snapshots = volume.join(garage::SNAPSHOTS_DIR);
    let newest = std::fs::read_dir(&snapshots)?
        .flatten()
        .filter(|entry| entry.path().join(garage::METADATA_FILE).is_file())
        .map(|entry| entry.file_name())
        .max()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "the store said it wrote a snapshot and none is there",
            )
        })?;
    let metadata = volume.join(garage::METADATA_DIR);
    std::fs::create_dir_all(&metadata)?;
    for journal in ["", "-wal", "-shm"] {
        let path = metadata.join(format!("{}{journal}", garage::METADATA_FILE));
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
    }
    std::fs::rename(
        snapshots.join(newest).join(garage::METADATA_FILE),
        metadata.join(garage::METADATA_FILE),
    )?;
    std::fs::remove_dir_all(snapshots)
}

/// A bucket of a restored copy, as Garage holds it in the backup's
/// metadata: its maximum, its CORS and its expiry. Never public, because a
/// public name is the original's, and with no key, by Jorge's decision: two
/// stores never open with one credential.
fn adopted(service_id: &str, name: BucketName, info: &serde_json::Value) -> Bucket {
    let bytes = info["bytes"].as_u64().unwrap_or(0);
    let max_size = info["quotas"]["maxSize"]
        .as_u64()
        .unwrap_or(bytes.max(1024 * 1024));
    let rules = |key: &str| {
        info[key]
            .as_array()
            .map(|rules| rules.to_vec())
            .unwrap_or_default()
    };
    let cors_origins = rules("corsRules")
        .iter()
        .flat_map(|rule| {
            rule["AllowedOrigin"]
                .as_array()
                .map(|origins| origins.to_vec())
                .unwrap_or_default()
        })
        .filter_map(|origin| {
            origin
                .as_str()
                .and_then(|text| CorsOrigin::parse(text).ok())
        })
        .collect();
    let lifecycle = rules("lifecycleRules");
    let days = |pointer: &str| {
        lifecycle
            .iter()
            .find_map(|rule| rule.pointer(pointer).and_then(|days| days.as_u64()))
            .and_then(|days| u32::try_from(days).ok())
            .filter(|days| *days > 0)
    };
    Bucket {
        id: format!("bkt-{}", wabot::prelude::password::generate(12)),
        service_id: service_id.to_string(),
        name,
        max_size,
        public: false,
        public_hostname: None,
        cors_origins,
        expire_days: days("/Expiration/Days"),
        abort_uploads_days: days("/AbortIncompleteMultipartUpload/DaysAfterInitiation"),
    }
}

/// Make rows of the buckets a restored copy holds, then forget where it was
/// restored from. Its keys are left to the convergence after this, which
/// deletes every key no row names — which is all of them.
///
/// From Garage rather than from the backup's copy of the node's database:
/// the snapshot is what the copy holds, and the rows have to say that.
pub async fn adopt(
    database: &SqliteDatabase,
    store: &ObjectStore,
    address: Ipv4Addr,
) -> Result<usize, String> {
    let admin = Admin::at(address, &store.admin_token);
    admin.wait_until_up().await?;
    let rows = crate::infra::repositories::object_stores::buckets(database, &store.service_id)
        .await
        .map_err(|error| error.to_string())?;
    let mut made = 0;
    for listed in admin.buckets().await? {
        let Some(name) = listed
            .global_aliases
            .iter()
            .find_map(|alias| BucketName::parse(alias).ok())
        else {
            continue;
        };
        if rows.iter().any(|row| row.name == name) {
            continue;
        }
        let info: serde_json::Value = admin
            .read(
                Method::GET,
                &format!("/v2/GetBucketInfo?id={}", listed.id),
                None,
            )
            .await?;
        let bucket = adopted(&store.service_id, name, &info);
        crate::infra::repositories::object_stores::create_bucket(database, &bucket)
            .await
            .map_err(|error| format!("its bucket {} does not fit: {error}", bucket.name))?;
        name_adopted(database, &store.service_id, &bucket.name).await;
        made += 1;
    }
    crate::infra::repositories::object_stores::set_restored_from(database, &store.service_id, None)
        .await
        .map_err(|error| error.to_string())?;
    Ok(made)
}

/// A bucket whose name does not reach a node yet is still a bucket, reached
/// by its path: the copy is not refused for a record somebody has not added.
async fn name_adopted(database: &SqliteDatabase, service_id: &str, bucket: &BucketName) {
    let named = match crate::infra::deploy::stores::bucket_name(database, service_id, bucket).await
    {
        Ok(Some(named)) => named,
        Ok(None) => return,
        Err(reason) => {
            tracing::warn!(bucket = %bucket.as_str(), %reason, "a restored bucket has no name of its own");
            return;
        }
    };
    if let Err(error) =
        crate::infra::deploy::stores::route_bucket_name(database, service_id, &named).await
    {
        tracing::warn!(bucket = %bucket.as_str(), %error, "a restored bucket has no name of its own");
    }
}

/// The address of the copy of this store that runs here, if one does.
pub async fn running_here(database: &SqliteDatabase, service_id: &str) -> Option<Ipv4Addr> {
    crate::infra::repositories::replicas::of_service(database, service_id)
        .await
        .ok()?
        .into_iter()
        .find(|replica| replica.is_here() && !replica.evicted())?
        .address?
        .parse()
        .ok()
}

pub async fn admin_for(database: &SqliteDatabase, service_id: &str) -> Result<Admin, String> {
    let store = crate::infra::repositories::object_stores::of_service(database, service_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("this service is not a store")?;
    let address = running_here(database, service_id)
        .await
        .ok_or("the store is not running on this node")?;
    Ok(Admin::at(address, &store.admin_token))
}

/// The pass, on the copy running here. `Ok(false)` when none runs: the next
/// deployment converges, and nothing is reachable until it does.
///
/// A pass that applied everything clears what the copy's row said, so the
/// sentence naming a lost key leaves with the key.
pub async fn converge_if_running(
    database: &SqliteDatabase,
    service_id: &str,
) -> Result<bool, String> {
    use crate::infra::repositories::replicas;
    let Some(address) = running_here(database, service_id).await else {
        return Ok(false);
    };
    let refused = |error: crate::domain::errors::PlatformError| error.to_string();
    let store = crate::infra::repositories::object_stores::of_service(database, service_id)
        .await
        .map_err(refused)?
        .ok_or("this service is not a store")?;
    let lost = converge(database, &store, address).await?;
    for replica in replicas::of_service(database, service_id)
        .await
        .map_err(refused)?
        .into_iter()
        .filter(|replica| replica.is_here() && replica.last_error != lost)
    {
        replicas::set_last_error(database, &replica.id, lost.as_deref())
            .await
            .map_err(refused)?;
    }
    Ok(true)
}

/// Make Garage hold exactly what the rows say: each bucket with its
/// settings, each key with its permissions, and no key the rows do not name.
///
/// A bucket Garage holds and no row names is left alone. It may hold objects,
/// and a backup older than the rows is how one appears; deleting it is a
/// decision this pass is not in a position to make.
///
/// **One pass at a time, reading the rows once Garage answers.** The pass a
/// deployment runs read them before waiting for Garage to start, which takes
/// half a minute; a key made in that time had its own pass, and then the
/// deployment's went on with the rows it had and deleted it. The page showed
/// a key no store held. With the rows read under the lock, the last pass to
/// run has every row written before it began.
///
/// **A key Garage deleted does not stop the rest**, and is not a failure of
/// the pass: `Ok(Some(_))` names it, for the copy's row. Such a row stopped
/// the pass at that key, every time, so no key after it and no bucket's
/// permissions were applied, a new key was rolled back, and the reason was
/// Garage's 409 without the key's name. A backup taken before a key was
/// revoked brings one back.
pub async fn converge(
    database: &SqliteDatabase,
    store: &ObjectStore,
    address: Ipv4Addr,
) -> Result<Option<String>, String> {
    static ONE_PASS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let admin = Admin::at(address, &store.admin_token);
    admin.wait_until_up().await?;
    let _pass = ONE_PASS.lock().await;

    let refused = |error: crate::domain::errors::PlatformError| error.to_string();
    let buckets = crate::infra::repositories::object_stores::buckets(database, &store.service_id)
        .await
        .map_err(refused)?;
    let keys =
        crate::infra::repositories::object_stores::keys_of_store(database, &store.service_id)
            .await
            .map_err(refused)?;

    let held = admin.buckets().await?;
    let mut ids = std::collections::BTreeMap::new();
    for bucket in &buckets {
        let listed = held.iter().find(|listed| {
            listed
                .global_aliases
                .iter()
                .any(|alias| alias == bucket.name.as_str())
        });
        let (id, aliases) = match listed {
            Some(listed) => (listed.id.clone(), listed.global_aliases.clone()),
            None => (
                admin.create_bucket(bucket.name.as_str()).await?,
                vec![bucket.name.as_str().to_string()],
            ),
        };
        let wanted: Vec<&str> = std::iter::once(bucket.name.as_str())
            .chain(bucket.public_hostname.as_deref())
            .collect();
        for alias in aliases
            .iter()
            .filter(|alias| !wanted.contains(&alias.as_str()))
        {
            admin.alias("/v2/RemoveBucketAlias", &id, alias).await?;
        }
        for alias in wanted
            .iter()
            .filter(|alias| !aliases.iter().any(|held| held == *alias))
        {
            admin.alias("/v2/AddBucketAlias", &id, alias).await?;
        }
        admin
            .call(
                Method::POST,
                &format!("/v2/UpdateBucket?id={id}"),
                Some(garage::bucket_settings(bucket)),
            )
            .await?;
        ids.insert(bucket.id.clone(), id);
    }

    let held_keys = admin.keys().await?;
    let mut lost = Vec::new();
    for key in &keys {
        if !held_keys.iter().any(|held| held.id == key.access_key_id)
            && !admin.give_back(key).await?
        {
            lost.push(key.access_key_id.as_str());
        }
    }
    for held in &held_keys {
        if !keys.iter().any(|key| key.access_key_id == held.id) {
            admin.delete_key(&held.id).await?;
        }
    }

    for bucket in &buckets {
        let Some(id) = ids.get(&bucket.id) else {
            continue;
        };
        let usage: Usage = admin
            .read(Method::GET, &format!("/v2/GetBucketInfo?id={id}"), None)
            .await?;
        for key in keys
            .iter()
            .filter(|key| key.bucket_id == bucket.id && !lost.contains(&key.access_key_id.as_str()))
        {
            let wanted = garage::permissions(key.access);
            let held = usage
                .keys
                .iter()
                .find(|holder| holder.access_key_id == key.access_key_id)
                .map(|holder| Permissions::from(holder.permissions))
                .unwrap_or(NOTHING);
            if held == wanted {
                continue;
            }
            admin
                .change("/v2/AllowBucketKey", id, &key.access_key_id, wanted)
                .await?;
            let excess = garage::excess(held, wanted);
            if excess.any() {
                admin
                    .change("/v2/DenyBucketKey", id, &key.access_key_id, excess)
                    .await?;
            }
        }
        for holder in usage.keys.iter().filter(|holder| {
            !keys
                .iter()
                .any(|key| key.bucket_id == bucket.id && key.access_key_id == holder.access_key_id)
        }) {
            let held = Permissions::from(holder.permissions);
            if held.any() {
                admin
                    .change("/v2/DenyBucketKey", id, &holder.access_key_id, held)
                    .await?;
            }
        }
    }
    if lost.is_empty() {
        return Ok(None);
    }
    let named: Vec<String> = keys
        .iter()
        .filter(|key| lost.contains(&key.access_key_id.as_str()))
        .map(|key| {
            let bucket = buckets
                .iter()
                .find(|bucket| bucket.id == key.bucket_id)
                .map_or("", |bucket| bucket.name.as_str());
            format!("“{}” of {bucket} ({})", key.name, key.access_key_id)
        })
        .collect();
    Ok(Some(format!(
        "Garage has deleted the key {} and never takes a deleted key back, so it opens \
         nothing: revoke it and make another.",
        named.join(", ")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &std::path::Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(path, text).expect("write");
    }

    #[test]
    fn a_restored_bucket_is_read_back_as_it_was_written_and_is_never_public() {
        let original = Bucket {
            id: "bkt-original".into(),
            service_id: "s-original".into(),
            name: BucketName::parse("uploads").expect("name"),
            max_size: 5 * 1024 * 1024 * 1024,
            public: true,
            public_hostname: Some("assets.shop.example.com".into()),
            cors_origins: CorsOrigin::parse_lines("https://shop.example.com\n*").expect("origins"),
            expire_days: Some(30),
            abort_uploads_days: Some(7),
        };
        let mut info = garage::bucket_settings(&original);
        info["bytes"] = serde_json::json!(1234);

        let restored = adopted("s-copy", original.name.clone(), &info);

        assert_eq!(
            Bucket {
                id: restored.id.clone(),
                ..original.clone()
            },
            Bucket {
                service_id: "s-original".into(),
                public: true,
                public_hostname: Some("assets.shop.example.com".into()),
                ..restored.clone()
            }
        );
        assert_eq!(restored.service_id, "s-copy");
        assert_eq!(restored.public_hostname, None);
        assert!(!restored.public);
        assert_ne!(restored.id, original.id);
    }

    #[test]
    fn a_restored_volume_leaves_the_originals_peers_behind_and_is_not_brought_twice() {
        let dir = tempfile::tempdir().expect("dir");
        let copy = dir.path().join("backup").join(garage::DATA_VOLUME);
        write(
            &copy.join(garage::METADATA_DIR).join(garage::METADATA_FILE),
            "metadata",
        );
        write(
            &copy.join(garage::METADATA_DIR).join(PEER_LIST),
            "the original's address",
        );
        write(&copy.join("data").join("ab").join("block"), "an object");
        let volume = dir
            .path()
            .join("volumes")
            .join("shop.copy")
            .join(garage::DATA_VOLUME);

        assert!(bring_volume(&copy, &volume).expect("brought"));
        assert_eq!(
            std::fs::read_to_string(
                volume
                    .join(garage::METADATA_DIR)
                    .join(garage::METADATA_FILE)
            )
            .expect("metadata"),
            "metadata"
        );
        assert!(volume.join("data").join("ab").join("block").exists());
        assert!(!volume.join(garage::METADATA_DIR).join(PEER_LIST).exists());

        write(
            &volume
                .join(garage::METADATA_DIR)
                .join(garage::METADATA_FILE),
            "written since",
        );
        assert!(!bring_volume(&copy, &volume).expect("already there"));
        assert_eq!(
            std::fs::read_to_string(
                volume
                    .join(garage::METADATA_DIR)
                    .join(garage::METADATA_FILE)
            )
            .expect("metadata"),
            "written since"
        );
    }

    #[test]
    fn a_backup_with_no_metadata_in_it_is_not_brought() {
        let dir = tempfile::tempdir().expect("dir");
        let copy = dir.path().join("backup").join(garage::DATA_VOLUME);
        write(&copy.join("data").join("block"), "an object");
        let volume = dir.path().join("volume");
        assert!(bring_volume(&copy, &volume).is_err());
        assert!(!volume.exists());
    }

    #[test]
    fn a_copied_store_opens_as_its_newest_snapshot_and_carries_no_journal() {
        let dir = tempfile::tempdir().expect("dir");
        let volume = dir.path();
        let meta = volume.join(garage::METADATA_DIR);
        let snapshots = volume.join(garage::SNAPSHOTS_DIR);
        write(&meta.join("db.sqlite"), "live");
        write(&meta.join("db.sqlite-wal"), "journal");
        write(&meta.join("node_key"), "identity");
        write(
            &snapshots.join("2026-11-16T10:00:00Z").join("db.sqlite"),
            "older",
        );
        write(
            &snapshots.join("2026-11-16T11:00:00Z").join("db.sqlite"),
            "newest",
        );

        carry_newest_snapshot(volume).expect("carried");

        assert_eq!(
            std::fs::read_to_string(meta.join("db.sqlite")).expect("db"),
            "newest"
        );
        assert!(!meta.join("db.sqlite-wal").exists());
        assert_eq!(
            std::fs::read_to_string(meta.join("node_key")).expect("key"),
            "identity"
        );
        assert!(!snapshots.exists());
    }

    #[test]
    fn a_copy_with_no_snapshot_in_it_is_refused_and_keeps_its_metadata() {
        let dir = tempfile::tempdir().expect("dir");
        let volume = dir.path();
        write(&volume.join(garage::METADATA_DIR).join("db.sqlite"), "live");
        std::fs::create_dir_all(volume.join(garage::SNAPSHOTS_DIR)).expect("dir");

        assert!(carry_newest_snapshot(volume).is_err());
        assert_eq!(
            std::fs::read_to_string(volume.join(garage::METADATA_DIR).join("db.sqlite"))
                .expect("db"),
            "live"
        );
    }
}
