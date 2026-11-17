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

use crate::domain::entities::bucket_key::BucketKey;
use crate::domain::entities::object_store::ObjectStore;
use crate::domain::policies::garage::{self, Permissions};

const TIMEOUT: Duration = Duration::from_secs(30);

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

    async fn give_back(&self, key: &BucketKey) -> Result<(), String> {
        self.call(
            Method::POST,
            "/v2/ImportKey",
            Some(serde_json::json!({
                "accessKeyId": key.access_key_id,
                "secretAccessKey": key.secret_key,
                "name": key.name,
            })),
        )
        .await
        .map(|_| ())
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
pub async fn converge_if_running(
    database: &SqliteDatabase,
    service_id: &str,
) -> Result<bool, String> {
    let Some(address) = running_here(database, service_id).await else {
        return Ok(false);
    };
    let store = crate::infra::repositories::object_stores::of_service(database, service_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("this service is not a store")?;
    converge(database, &store, address).await.map(|_| true)
}

/// Make Garage hold exactly what the rows say: each bucket with its
/// settings, each key with its permissions, and no key the rows do not name.
///
/// A bucket Garage holds and no row names is left alone. It may hold objects,
/// and a backup older than the rows is how one appears; deleting it is a
/// decision this pass is not in a position to make.
pub async fn converge(
    database: &SqliteDatabase,
    store: &ObjectStore,
    address: Ipv4Addr,
) -> Result<(), String> {
    let refused = |error: crate::domain::errors::PlatformError| error.to_string();
    let buckets = crate::infra::repositories::object_stores::buckets(database, &store.service_id)
        .await
        .map_err(refused)?;
    let keys =
        crate::infra::repositories::object_stores::keys_of_store(database, &store.service_id)
            .await
            .map_err(refused)?;
    let admin = Admin::at(address, &store.admin_token);
    admin.wait_until_up().await?;

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
    for key in &keys {
        if !held_keys.iter().any(|held| held.id == key.access_key_id) {
            admin.give_back(key).await?;
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
        for key in keys.iter().filter(|key| key.bucket_id == bucket.id) {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &std::path::Path, text: &str) {
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        std::fs::write(path, text).expect("write");
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
