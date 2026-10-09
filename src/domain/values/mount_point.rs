use std::fmt;

use crate::domain::errors::{PlatformError, PlatformResult};

/// A destination the node can mount over without breaking the container.
///
/// Parsed once and immutable after: trailing slashes are trimmed, so
/// `/data` and `/data/` are one path rather than two rows the unique
/// index would let through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountPoint(String);

/// Where a container may not be given a directory.
///
/// Every one of these is a mount the container needs to be a container
/// — see `runtime::spec` — and a volume is appended *after* them, so it
/// would win. Losing `/proc` is a container that does not start;
/// losing `/etc/resolv.conf` is one that starts and cannot resolve
/// anything, which is worse because it looks like the image's fault.
const RESERVED: [&str; 9] = [
    "/",
    "/proc",
    "/dev",
    "/dev/pts",
    "/dev/shm",
    "/dev/mqueue",
    "/sys",
    "/etc/resolv.conf",
    // The node writes this one too, from the rows — see `deploy::hosts`.
    // A volume over it is a container that can reach nothing by name.
    "/etc/hosts",
];

impl MountPoint {
    pub fn parse(typed: &str) -> PlatformResult<Self> {
        let path = typed.trim().trim_end_matches('/');
        let path = match path.is_empty() {
            true => "/",
            false => path,
        };

        if !path.starts_with('/') {
            return Err(PlatformError::Refused(format!(
                "a mount point is an absolute path: {path:?} — try /var/lib/postgresql/data"
            )));
        }
        // `..` is refused rather than resolved. The destination is
        // interpreted inside the container's own root by the runtime, so
        // a traversal cannot reach the node's filesystem — but a path
        // nobody can predict from reading it is one nobody can audit
        // either.
        if path.split('/').any(|part| part == "..") {
            return Err(PlatformError::Refused(
                "a mount point has no `..` in it".into(),
            ));
        }
        if RESERVED.contains(&path) {
            return Err(PlatformError::Refused(format!(
                "{path} is one of the mounts a container needs to run, so nothing may be \
                 mounted over it"
            )));
        }
        Ok(Self(path.to_string()))
    }
}

impl fmt::Display for MountPoint {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        out.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailing_slash_is_the_same_path() {
        assert_eq!(
            MountPoint::parse("/data/").expect("parsed"),
            MountPoint::parse("/data").expect("parsed")
        );
    }

    #[test]
    fn a_relative_path_is_refused() {
        assert!(MountPoint::parse("data").is_err());
    }

    /// Refused rather than resolved: a path nobody can predict from
    /// reading it is one nobody can audit.
    #[test]
    fn a_traversal_is_refused() {
        assert!(MountPoint::parse("/var/../etc").is_err());
    }

    #[test]
    fn nothing_mounts_over_what_a_container_needs() {
        for reserved in ["/", "/proc", "/etc/hosts"] {
            assert!(MountPoint::parse(reserved).is_err(), "{reserved}");
        }
    }
}
