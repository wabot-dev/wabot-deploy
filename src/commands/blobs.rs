//! The part of a backup every node shares.
//!
//! ## One directory for the whole network
//!
//! Every node points at the same backup root, so what is per-node lives
//! under its own id and what is not lives once. Image layers are the
//! second kind: two nodes running the same base image hold the same
//! bytes, and a backup that copied them per node would store the
//! network's images as many times as there are machines.
//!
//! ## The name is the hash, which is the whole of the deduplication
//!
//! An OCI blob is addressed by the sha256 of its contents. So a file
//! that is already there under a given digest **is** the blob for that
//! digest — there is nothing to compare, nothing to reconcile, and no
//! way for two nodes to write conflicting versions of the same name.
//! Skipping what exists is not an optimisation here, it is the
//! definition.
//!
//! That also makes writing safe from several nodes at once, which a
//! shared root has to be: the worst two simultaneous writers can do is
//! write identical bytes.
//!
//! ## Written through a temporary name
//!
//! A reader that finds `blobs/sha256/<digest>` trusts it completely — it
//! cannot check the hash without reading the whole thing, and the whole
//! point is not to. So a half-written file under the real name would be
//! a blob that is quietly corrupt for ever. Write beside it, then
//! rename, which is atomic within a filesystem.

use std::path::{Path, PathBuf};

/// Where a blob of this digest lives under the backup root.
///
/// `sha256/<hex>`, which is the layout of an OCI registry and of
/// containerd's own content store. Somebody who has seen either can read
/// this directory without being told what it is.
pub fn path(root: &Path, digest: &str) -> Option<PathBuf> {
    let (algorithm, hex) = digest.split_once(':')?;
    // A digest is what names a file here, so what may name a file is
    // exactly what a digest may be. Anything else is refused rather than
    // sanitised: `..` in a digest is not a blob with an odd name, it is
    // something trying to write outside the store.
    if algorithm != "sha256" || hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(root.join("blobs").join(algorithm).join(hex))
}

/// Whether this blob is already kept.
pub fn have(root: &Path, digest: &str) -> bool {
    path(root, digest).is_some_and(|path| path.exists())
}

/// Keep these bytes under their digest, unless they are already there.
///
/// Returns whether it wrote — so a caller can say how much of a backup
/// was new, which on the second node of a network is most of the
/// interesting number.
pub fn put(root: &Path, digest: &str, bytes: &[u8]) -> std::io::Result<bool> {
    let Some(path) = path(root, digest) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{digest:?} is not a digest this store can name"),
        ));
    };
    if path.exists() {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Beside it, then renamed: a reader trusts the name completely, so
    // a half-written file under the real one would be a blob that is
    // quietly corrupt for ever. The temporary name carries the process
    // id so that two nodes writing the same blob at once cannot land on
    // one another's partial file.
    let staging = path.with_extension(format!("part{}", std::process::id()));
    std::fs::write(&staging, bytes)?;
    match std::fs::rename(&staging, &path) {
        Ok(()) => Ok(true),
        // Somebody else finished first, which is fine: the name is the
        // hash, so their bytes are these bytes.
        Err(_) if path.exists() => {
            let _ = std::fs::remove_file(&staging);
            Ok(false)
        }
        Err(error) => {
            let _ = std::fs::remove_file(&staging);
            Err(error)
        }
    }
}

/// Read one back.
///
/// The restore half, which does not exist yet — a node being rebuilt
/// reads these back into its own content store and recreates the image
/// records from the manifest's `images`. Kept here rather than written
/// when that lands, because the pair is what makes the store make
/// sense: a `put` with no `get` is a directory nobody can explain.
#[allow(dead_code)]
pub fn get(root: &Path, digest: &str) -> std::io::Result<Vec<u8>> {
    let Some(path) = path(root, digest) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{digest:?} is not a digest this store can name"),
        ));
    };
    std::fs::read(path)
}

/// Everything an image is made of, as digests.
///
/// **The whole tree, or the image is not there.** A manifest names a
/// config and some layers; an index names manifests, one per platform.
/// Missing any one of them is an image that restores and will not run —
/// and it fails at `docker pull` time on a rebuilt node, which is the
/// worst moment to find out something was not copied.
///
/// Walked here rather than trusting `diff_ids` or the layer list from
/// the config, because those describe the *unpacked* image. What has to
/// be kept is what the registry serves, which is these descriptors.
pub async fn tree_of(
    client: &crate::runtime::client::Containerd,
    reference: &str,
) -> Result<Vec<(String, i64)>, String> {
    let target = crate::runtime::images::image_target(client, reference)
        .await
        .map_err(|error| error.to_string())?;

    let mut wanted = vec![(target.digest.clone(), target.size)];
    let mut queue = vec![target];

    // Breadth first through whatever is a manifest or an index. Blobs
    // that are neither — a config, a layer — are kept and not opened.
    while let Some(descriptor) = queue.pop() {
        if !is_manifest(&descriptor.media_type) {
            continue;
        }
        let bytes = crate::runtime::content::read(client, &descriptor)
            .await
            .map_err(|error| format!("{}: {error}", descriptor.digest))?;
        let Ok(document) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };

        // An index lists manifests; a manifest lists a config and
        // layers. Reading both keys rather than branching on the media
        // type: a document has one or the other, and this way an index
        // that also carried a config would not lose it.
        for key in ["manifests", "layers"] {
            for child in document
                .get(key)
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(found) = descriptor_of(child) {
                    wanted.push((found.digest.clone(), found.size));
                    queue.push(found);
                }
            }
        }
        if let Some(config) = document.get("config").and_then(descriptor_of) {
            wanted.push((config.digest.clone(), config.size));
        }
    }

    wanted.sort();
    wanted.dedup();
    Ok(wanted)
}

/// Whether this is something to open rather than only to keep.
fn is_manifest(media_type: &str) -> bool {
    media_type.contains("manifest") || media_type.contains("image.index")
}

fn descriptor_of(value: &serde_json::Value) -> Option<containerd_client::types::Descriptor> {
    Some(containerd_client::types::Descriptor {
        media_type: value.get("mediaType")?.as_str()?.to_string(),
        digest: value.get("digest")?.as_str()?.to_string(),
        size: value.get("size")?.as_i64()?,
        annotations: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The name is the hash, so writing the same digest twice is one
    /// file — which is the whole of what makes a shared backup root
    /// work. Two nodes with the same base image store it once.
    #[test]
    fn the_same_blob_from_two_nodes_is_kept_once() {
        let root = tempfile::tempdir().expect("tempdir");
        let digest = format!("sha256:{}", "a".repeat(64));

        assert!(!have(root.path(), &digest));
        assert!(put(root.path(), &digest, b"layer").expect("write"), "new");
        assert!(have(root.path(), &digest));

        // The second node's copy of the same layer writes nothing.
        assert!(
            !put(root.path(), &digest, b"layer").expect("write"),
            "already kept"
        );
        assert_eq!(get(root.path(), &digest).expect("read"), b"layer");
    }

    /// A digest names a file, so what may be a digest is exactly what
    /// may name one. `..` is refused rather than sanitised: it is not a
    /// blob with an odd name, it is something trying to write outside
    /// the store — and a shared root is written by more than one
    /// machine.
    #[test]
    fn only_something_shaped_like_a_digest_may_name_a_file() {
        let root = tempfile::tempdir().expect("tempdir");

        for bad in [
            "sha256:../../etc/passwd",
            "sha256:short",
            "md5:0123456789abcdef0123456789abcdef",
            "nocolon",
            "sha256:",
            &format!("sha256:{}", "g".repeat(64)),
        ] {
            assert!(path(root.path(), bad).is_none(), "{bad:?}");
            assert!(put(root.path(), bad, b"x").is_err(), "{bad:?}");
        }

        // And the shape it does accept is the one containerd writes.
        let good = format!("sha256:{}", "0".repeat(64));
        assert!(path(root.path(), &good).is_some());
    }

    /// Nothing is left under the real name unless it is complete. A
    /// reader cannot check the hash without reading the whole blob —
    /// and not doing that is the point — so a partial file under the
    /// real name would be corruption nothing ever notices.
    #[test]
    fn a_partial_write_never_wears_the_real_name() {
        let root = tempfile::tempdir().expect("tempdir");
        let digest = format!("sha256:{}", "b".repeat(64));
        put(root.path(), &digest, b"whole").expect("write");

        let kept = path(root.path(), &digest).expect("a path");
        let staging: Vec<_> = std::fs::read_dir(kept.parent().expect("parent"))
            .expect("read")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().contains(".part"))
            .collect();
        assert!(staging.is_empty(), "a temporary file was left behind");
    }
}
