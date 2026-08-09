//! Starting and stopping containers.
//!
//! ## The order is the whole thing
//!
//! 1. Pull and unpack the image, or the snapshot it needs will not exist.
//! 2. Read its config, or the container has nothing to run.
//! 3. Prepare a snapshot from the image's chain ID → the rootfs mounts.
//! 4. `Containers.Create` with the spec **and the runtime options** —
//!    this is where crun gets chosen.
//! 5. `Tasks.Create` with those mounts, `Tasks.Start`.
//!
//! Skip step 3 and the task fails with a missing rootfs. Skip the
//! options in step 4 and it starts under runc, which is not installed.
//! Both failures happen inside the shim, where the message says neither.
//!
//! ## Teardown is best-effort and ordered
//!
//! Task, then container, then snapshot — each holding a reference to
//! the next. Every step tolerates "already gone", because the common
//! reason to be tearing down is that something half-failed.

use containerd_client::services::v1::container::Runtime;
use containerd_client::services::v1::containers_client::ContainersClient;
use containerd_client::services::v1::tasks_client::TasksClient;
use containerd_client::services::v1::{
    Container, CreateContainerRequest, CreateTaskRequest, DeleteContainerRequest,
    DeleteTaskRequest, GetRequest, KillRequest, StartRequest,
};

use super::client::{ClientError, ClientResult, Containerd, RuncOptions, RUNTIME};
use super::spec::ContainerRequest;

/// What a running container looks like from outside.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStatus {
    pub pid: u32,
    /// containerd's own word: `CREATED`, `RUNNING`, `STOPPED`, …
    pub status: String,
    pub exit_code: u32,
}

impl TaskStatus {
    pub fn running(&self) -> bool {
        self.status == "RUNNING"
    }
}

/// The snapshot key for a container. Its own id, so the two are
/// findable from each other by an operator with `ctr`.
fn snapshot_key(id: &str) -> String {
    id.to_string()
}

/// Create and start a container from `image`.
///
/// Idempotent to the extent that matters: an existing container with
/// this id is torn down first, because the reason to deploy again is
/// that the old one should stop being the one serving.
pub async fn run(
    client: &Containerd,
    id: &str,
    image: &str,
    request: &ContainerRequest,
    // Not part of `ContainerRequest`: that is what the container will
    // *be*, and this is how its image is fetched. A registry credential
    // has no business in an OCI spec.
    credential: Option<&super::images::Credential>,
) -> ClientResult<TaskStatus> {
    // Whatever is there under this id is the previous deployment.
    remove(client, id).await?;

    super::images::ensure(client, image, credential).await?;
    let config = super::images::config(client, image).await?;
    let diff_ids = super::images::diff_ids(client, image).await?;

    let chain = super::snapshots::chain_id(&diff_ids).ok_or_else(|| {
        ClientError::Other(format!("{image} has no layers, so there is nothing to run"))
    })?;
    let mounts = super::snapshots::prepare(client, &snapshot_key(id), &chain).await?;

    let spec = super::spec::build(&config, request)
        .map_err(|error| ClientError::Other(error.to_string()))?;
    let spec_any =
        super::spec::to_any(&spec).map_err(|error| ClientError::Other(error.to_string()))?;

    ContainersClient::new(client.channel())
        .create(client.request(CreateContainerRequest {
            container: Some(Container {
                id: id.to_string(),
                image: image.to_string(),
                runtime: Some(Runtime {
                    name: RUNTIME.to_string(),
                    // The reason crun runs this and not runc. Omitting
                    // it is a container that starts under the default
                    // runtime, silently.
                    options: Some(RuncOptions::crun().to_any()),
                }),
                spec: Some(spec_any),
                snapshotter: super::snapshots::SNAPSHOTTER.to_string(),
                snapshot_key: snapshot_key(id),
                ..Default::default()
            }),
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Containers.Create",
            source,
        })?;

    TasksClient::new(client.channel())
        .create(client.request(CreateTaskRequest {
            container_id: id.to_string(),
            rootfs: mounts,
            // No stdio paths. containerd defaults to discarding, which
            // is right until logs are a feature: a FIFO nobody reads
            // fills and blocks the container's first write.
            ..Default::default()
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Tasks.Create",
            source,
        })?;

    TasksClient::new(client.channel())
        .start(client.request(StartRequest {
            container_id: id.to_string(),
            ..Default::default()
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Tasks.Start",
            source,
        })?;

    tracing::info!(container = id, %image, "started");
    status(client, id)
        .await?
        .ok_or_else(|| ClientError::Other(format!("{id} started and then vanished")))
}

/// What containerd says about a container's task, or `None` if it has
/// none.
pub async fn status(client: &Containerd, id: &str) -> ClientResult<Option<TaskStatus>> {
    match TasksClient::new(client.channel())
        .get(client.request(GetRequest {
            container_id: id.to_string(),
            ..Default::default()
        }))
        .await
    {
        Ok(response) => Ok(response.into_inner().process.map(|process| TaskStatus {
            pid: process.pid,
            status: process.status().as_str_name().to_string(),
            exit_code: process.exit_status,
        })),
        Err(status) if status.code() == tonic::Code::NotFound => Ok(None),
        Err(source) => Err(ClientError::Call {
            call: "Tasks.Get",
            source,
        }),
    }
}

/// Stop a container, and give it a chance to stop cleanly.
///
/// SIGTERM, wait, then SIGKILL. Skipping the wait is how a database
/// loses its last write, and skipping the SIGKILL is how a container
/// that ignores signals never goes away.
pub async fn stop(client: &Containerd, id: &str, grace: std::time::Duration) -> ClientResult<()> {
    if status(client, id).await?.is_none() {
        return Ok(());
    }

    kill(client, id, 15).await?;

    let deadline = std::time::Instant::now() + grace;
    while std::time::Instant::now() < deadline {
        match status(client, id).await? {
            None => return Ok(()),
            Some(task) if !task.running() => return Ok(()),
            Some(_) => tokio::time::sleep(std::time::Duration::from_millis(200)).await,
        }
    }

    tracing::warn!(container = id, ?grace, "did not stop; killing");
    kill(client, id, 9).await
}

async fn kill(client: &Containerd, id: &str, signal: u32) -> ClientResult<()> {
    match TasksClient::new(client.channel())
        .kill(client.request(KillRequest {
            container_id: id.to_string(),
            signal,
            all: true,
            ..Default::default()
        }))
        .await
    {
        Ok(_) => Ok(()),
        // Already gone between the check and the signal.
        Err(status) if status.code() == tonic::Code::NotFound => Ok(()),
        Err(source) => Err(ClientError::Call {
            call: "Tasks.Kill",
            source,
        }),
    }
}

/// Remove a container and everything it holds.
///
/// Ordered task → container → snapshot, because each holds a reference
/// to the next, and tolerant of absence at every step: the usual reason
/// to be here is that something half-failed.
pub async fn remove(client: &Containerd, id: &str) -> ClientResult<()> {
    if status(client, id).await?.is_some() {
        stop(client, id, std::time::Duration::from_secs(10)).await?;

        match TasksClient::new(client.channel())
            .delete(client.request(DeleteTaskRequest {
                container_id: id.to_string(),
            }))
            .await
        {
            Ok(_) => {}
            Err(status) if status.code() == tonic::Code::NotFound => {}
            Err(source) => {
                return Err(ClientError::Call {
                    call: "Tasks.Delete",
                    source,
                })
            }
        }
    }

    match ContainersClient::new(client.channel())
        .delete(client.request(DeleteContainerRequest { id: id.to_string() }))
        .await
    {
        Ok(_) => {}
        Err(status) if status.code() == tonic::Code::NotFound => {}
        Err(source) => {
            return Err(ClientError::Call {
                call: "Containers.Delete",
                source,
            })
        }
    }

    // Last, and after the container that referenced it: containerd
    // refuses to remove a snapshot still in use, and the error names
    // the snapshot rather than the container holding it.
    super::snapshots::remove(client, &snapshot_key(id)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The snapshot key is the container id, so an operator looking at
    /// `ctr snapshots ls` can tell which container a layer belongs to.
    #[test]
    fn the_snapshot_is_named_after_its_container() {
        assert_eq!(snapshot_key("svc-api"), "svc-api");
    }

    #[test]
    fn only_running_counts_as_running() {
        let task = |status: &str| TaskStatus {
            pid: 1,
            status: status.into(),
            exit_code: 0,
        };
        assert!(task("RUNNING").running());
        assert!(!task("CREATED").running(), "created is not yet serving");
        assert!(!task("STOPPED").running());
        assert!(!task("PAUSED").running());
    }
}
