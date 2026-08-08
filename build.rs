//! One job: give the vendored assets an ETag that moves when they do.
//!
//! They used to be tagged with the crate version, on the reasoning that
//! "assets only change when the binary does". That is true and useless:
//! the *version* does not change when the binary does. Four deploys of
//! 0.1.5 shipped four different `console.js` files under one ETag, and
//! because the assets are served `must-revalidate`, every browser that
//! had seen an earlier one asked, was told 304, and kept the old file.
//! The symptom was a feature that worked on the server and did nothing
//! in the page — with no error anywhere, because nothing failed.
//!
//! So the tag is a hash of the bytes. FNV-1a rather than a crate: it is
//! twelve lines, this is not a security boundary, and a build
//! dependency to name a cache entry is a poor trade.

use std::path::Path;

fn main() {
    let assets = Path::new("assets");
    println!("cargo:rerun-if-changed={}", assets.display());

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for path in files(assets) {
        // The name is hashed too, so moving a file changes the tag even
        // when the bytes did not.
        for byte in path
            .to_string_lossy()
            .as_bytes()
            .iter()
            .chain(std::fs::read(&path).unwrap_or_default().iter())
        {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100_0000_01b3);
        }
    }

    println!("cargo:rustc-env=WABOT_ASSET_HASH={hash:016x}");
}

/// Every file under `dir`, sorted, so the hash does not depend on the
/// order the filesystem happens to hand them back.
fn files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files(&path));
        } else {
            found.push(path);
        }
    }
    found.sort();
    found
}
