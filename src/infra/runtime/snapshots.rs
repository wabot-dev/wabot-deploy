//! The container's root filesystem.
//!
//! ## The chain ID is computed, not looked up
//!
//! An unpacked image's snapshot is keyed by its *chain ID*, and nothing
//! in the containerd API hands one over. It is derived from the
//! uncompressed layer digests in the image config:
//!
//! ```text
//! chain[0] = diff_ids[0]
//! chain[i] = sha256("chain[i-1] diff_ids[i]")     // one space
//! ```
//!
//! That single space is the whole format. Get it wrong and
//! `Snapshots.Prepare` answers `NotFound` for a parent that is right
//! there, which reads as "the image was not unpacked" and sends you
//! looking in the wrong place.
//!
//! ## Prepare, not View
//!
//! `Prepare` creates a writable snapshot on top of the image's layers —
//! the container's own upper layer. `View` is read-only, which is right
//! for inspecting an image and wrong for running one.

use containerd_client::services::v1::snapshots::{
    snapshots_client::SnapshotsClient, CommitSnapshotRequest, PrepareSnapshotRequest,
    RemoveSnapshotRequest, StatSnapshotRequest,
};
use containerd_client::types::Mount;

use super::client::{ClientError, ClientResult, Containerd};

/// overlayfs, which is what the preflight checks for and what the
/// pull unpacks into.
pub const SNAPSHOTTER: &str = "overlayfs";

/// The chain ID of an image's topmost layer.
///
/// `diff_ids` are the *uncompressed* digests from the image config's
/// `rootfs`, in order. Returns `None` for an image with no layers,
/// which is legal and cannot be run.
pub fn chain_id(diff_ids: &[String]) -> Option<String> {
    use sha2::{Digest, Sha256};

    let mut chain = diff_ids.first()?.clone();
    for diff_id in &diff_ids[1..] {
        // The separator is one space. containerd's `identity.ChainID`
        // is `sha256(parent + " " + layer)`, and the digest is written
        // with its algorithm prefix.
        let digest = Sha256::digest(format!("{chain} {diff_id}").as_bytes());
        chain = format!(
            "sha256:{}",
            digest
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>()
        );
    }
    Some(chain)
}

/// Create the container's writable layer and return its mounts.
///
/// The mounts go straight into `CreateTaskRequest.rootfs`; for
/// overlayfs they are the `lowerdir=…,upperdir=…,workdir=…` options
/// the kernel needs.
pub async fn prepare(
    client: &Containerd,
    key: &str,
    parent_chain_id: &str,
) -> ClientResult<Vec<Mount>> {
    let response = SnapshotsClient::new(client.channel())
        .prepare(client.request(PrepareSnapshotRequest {
            snapshotter: SNAPSHOTTER.to_string(),
            key: key.to_string(),
            parent: parent_chain_id.to_string(),
            labels: Default::default(),
        }))
        .await
        .map_err(|source| match source.code() {
            // The message containerd sends here names a digest and
            // nothing else, which sends people looking for a broken
            // pull. Say what it actually means.
            tonic::Code::NotFound => ClientError::Other(format!(
                "no unpacked layer {parent_chain_id} in the {SNAPSHOTTER} snapshotter — \
                 the image is in the content store but was not unpacked, or the chain ID \
                 was computed wrongly"
            )),
            _ => ClientError::Call {
                call: "Snapshots.Prepare",
                source,
            },
        })?
        .into_inner();

    Ok(response.mounts)
}

/// Does this snapshot exist?
///
/// Asked before unpacking a layer: on a node with one content store,
/// most layers of most images are already there because something else
/// unpacked them.
pub async fn exists(client: &Containerd, key: &str) -> ClientResult<bool> {
    match SnapshotsClient::new(client.channel())
        .stat(client.request(StatSnapshotRequest {
            snapshotter: SNAPSHOTTER.to_string(),
            key: key.to_string(),
        }))
        .await
    {
        Ok(_) => Ok(true),
        Err(status) if status.code() == tonic::Code::NotFound => Ok(false),
        Err(source) => Err(ClientError::Call {
            call: "Snapshots.Stat",
            source,
        }),
    }
}

/// Prepare a snapshot without the container-id naming.
///
/// [`prepare`] builds a container's writable layer; this builds the
/// intermediate one an unpack applies a layer onto. Same call, and
/// kept separate so the error message about a missing parent stays
/// attached to the case where it means something.
pub async fn prepare_from(
    client: &Containerd,
    key: &str,
    parent: &str,
) -> ClientResult<Vec<Mount>> {
    let response = SnapshotsClient::new(client.channel())
        .prepare(client.request(PrepareSnapshotRequest {
            snapshotter: SNAPSHOTTER.to_string(),
            key: key.to_string(),
            parent: parent.to_string(),
            labels: Default::default(),
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Snapshots.Prepare",
            source,
        })?
        .into_inner();

    Ok(response.mounts)
}

/// Turn a prepared snapshot into a permanent one, named by its chain
/// ID — which is what everything else looks it up by.
pub async fn commit(client: &Containerd, chain_id: &str, key: &str) -> ClientResult<()> {
    SnapshotsClient::new(client.channel())
        .commit(client.request(CommitSnapshotRequest {
            snapshotter: SNAPSHOTTER.to_string(),
            name: chain_id.to_string(),
            key: key.to_string(),
            labels: Default::default(),
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Snapshots.Commit",
            source,
        })?;
    Ok(())
}

/// Remove a snapshot. A missing one is already removed.
pub async fn remove(client: &Containerd, key: &str) -> ClientResult<()> {
    match SnapshotsClient::new(client.channel())
        .remove(client.request(RemoveSnapshotRequest {
            snapshotter: SNAPSHOTTER.to_string(),
            key: key.to_string(),
        }))
        .await
    {
        Ok(_) => Ok(()),
        Err(status) if status.code() == tonic::Code::NotFound => Ok(()),
        Err(source) => Err(ClientError::Call {
            call: "Snapshots.Remove",
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single-layer image's chain ID *is* its only diff id — not a
    /// hash of it. Hashing a single layer produces a key containerd
    /// has never heard of.
    #[test]
    fn one_layer_is_its_own_chain_id() {
        let only = "sha256:aaaa".to_string();
        assert_eq!(chain_id(std::slice::from_ref(&only)), Some(only));
    }

    /// The value containerd computes, checked against the algorithm
    /// rather than against a number this code produced.
    #[test]
    fn two_layers_hash_with_a_single_space() {
        use sha2::{Digest, Sha256};

        let ids = vec!["sha256:aaaa".to_string(), "sha256:bbbb".to_string()];
        let expected = format!(
            "sha256:{:x}",
            Sha256::digest(b"sha256:aaaa sha256:bbbb" as &[u8])
        );
        assert_eq!(chain_id(&ids), Some(expected));
    }

    /// And it folds left, so a third layer hashes against the *chain*
    /// so far rather than against the previous diff id.
    #[test]
    fn three_layers_fold_left() {
        use sha2::{Digest, Sha256};

        let ids = vec![
            "sha256:a".to_string(),
            "sha256:b".to_string(),
            "sha256:c".to_string(),
        ];
        let first = format!("sha256:{:x}", Sha256::digest(b"sha256:a sha256:b" as &[u8]));
        let second = format!(
            "sha256:{:x}",
            Sha256::digest(format!("{first} sha256:c").as_bytes())
        );
        assert_eq!(chain_id(&ids), Some(second));

        // The wrong fold — hashing consecutive diff ids — is a
        // plausible mistake that produces a key containerd does not
        // have, so pin that it is *not* what happens.
        let wrong = format!("sha256:{:x}", Sha256::digest(b"sha256:b sha256:c" as &[u8]));
        assert_ne!(chain_id(&ids), Some(wrong));
    }

    #[test]
    fn no_layers_has_no_chain() {
        assert_eq!(chain_id(&[]), None);
    }
}
