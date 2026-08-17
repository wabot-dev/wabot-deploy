//! Where a backup goes when it does not stay here.
//!
//! `backup` has always ended by saying that a copy on the same disk
//! protects against nothing that has ever happened to a disk. It could
//! not do anything about it: `--out` took a path, and moving the result
//! was the operator's errand. This is that errand.
//!
//! ## Two schemes, and everything else is a path
//!
//! `ssh://[user@]host/path` and `s3://bucket/prefix`. Anything without a
//! scheme is a local directory.
//!
//! Deliberately *not* the `user@host:/path` form that `scp` accepts,
//! however familiar it is. A destination is the one argument where being
//! clever about ambiguity is expensive: a local path that happens to
//! contain a colon would become a hostname, and the operator would find
//! out when they went looking for the backup that was never written
//! here. An unrecognised scheme is a path, and a path is never a host.
//!
//! ## Built here, then sent
//!
//! Not streamed. Three of the things a backup contains cannot be
//! produced without a filesystem — `VACUUM INTO` wants a file,
//! `pg_basebackup` wants a directory, and the blobs are read out of
//! containerd — so the choice is between a local staging copy and
//! reimplementing all three against a remote. The staging copy is
//! removed when the send succeeds and **kept when it does not**, which
//! is the way round that leaves somebody holding their bytes.
//!
//! ## The name being the hash is what makes the transfer cheap
//!
//! A shared backup root holds one copy of each image layer for the whole
//! network — see `commands::blobs`. Sending it again per node would
//! undo that, so the transfer has to skip what is already there.
//!
//! Over SSH that is `rsync --ignore-existing`, and this is the rare case
//! where that flag is exactly right rather than nearly right: a file
//! present under a given digest **is** the blob for that digest, so
//! there is nothing to compare and no version to be stale. Everything
//! that is not a blob is sent normally, because a manifest for this
//! run is new by definition.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Where the bytes are meant to end up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// A directory on this machine. What `--out` has always meant.
    Local(PathBuf),
    /// A directory on another machine, reached with the operator's own
    /// SSH configuration — keys, `~/.ssh/config`, agent and all. This
    /// node holds no credential of its own for it, which is the point:
    /// the thing that can read every backup in the network should not be
    /// a secret sitting on each node.
    Ssh {
        /// `[user@]host`, passed to `ssh` and `rsync` untouched so that
        /// a `Host` alias in the operator's config works.
        host: String,
        path: String,
    },
    S3 {
        bucket: String,
        /// May be empty, meaning the root of the bucket.
        prefix: String,
    },
}

impl Destination {
    /// Read a destination as an operator would have written it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err("an empty destination".into());
        }

        if let Some(rest) = text.strip_prefix("ssh://") {
            let (host, path) = rest
                .split_once('/')
                .ok_or_else(|| format!("{text:?} names a host and no directory on it"))?;
            if host.is_empty() {
                return Err(format!("{text:?} names no host"));
            }
            // The leading slash belongs to the path: `ssh://host/srv/x`
            // is `/srv/x`, the way a URL reads. A relative directory on
            // the far side is written `ssh://host/./x`, which is what
            // rsync itself would want.
            return Ok(Self::Ssh {
                host: host.to_string(),
                path: format!("/{path}"),
            });
        }

        if let Some(rest) = text.strip_prefix("s3://") {
            let (bucket, prefix) = match rest.split_once('/') {
                Some((bucket, prefix)) => (bucket, prefix.trim_end_matches('/')),
                None => (rest, ""),
            };
            if bucket.is_empty() {
                return Err(format!("{text:?} names no bucket"));
            }
            return Ok(Self::S3 {
                bucket: bucket.to_string(),
                prefix: prefix.to_string(),
            });
        }

        // Anything that *looks* like a scheme is refused rather than
        // treated as a path. Somebody who wrote `sftp://host/x` — or
        // `s3:/bucket`, one slash short — meant a remote, and would
        // otherwise get a local directory named `s3:` on the very
        // machine they were trying to get the backup off. That failure
        // is silent until the day the backup is needed.
        //
        // Only the first segment is examined, which is what lets a real
        // path keep its colons: `/mnt/disk:1/backups` has none before
        // its first slash. A *relative* path whose first segment has one
        // is the price — write `./backups:2026` and it is a path again.
        let head = text.split('/').next().unwrap_or(text);
        if let Some((scheme, _)) = head.split_once(':') {
            if !scheme.is_empty()
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
            {
                return Err(format!(
                    "{text:?} is not a destination this knows. Use ssh://[user@]host/path, \
                     s3://bucket/prefix, or a plain path — and note both slashes."
                ));
            }
        }

        Ok(Self::Local(PathBuf::from(text)))
    }

    /// Does this leave the machine?
    pub fn is_remote(&self) -> bool {
        !matches!(self, Self::Local(_))
    }

    /// Everything that can be checked before a byte is written.
    ///
    /// **Asked first, for the same reason the parse is.** A missing
    /// `rsync` is the kind of thing that is true before the backup
    /// starts and stays true for the twenty minutes it takes — and
    /// finding out *after* `pg_basebackup` has copied every database is
    /// the whole cost of the run thrown away to learn a fact that was
    /// available at the start.
    ///
    /// This was the shape of the bug: the tool check lived at the moment
    /// of use, which for a send is the very end.
    pub fn preflight(&self) -> Result<(), String> {
        match self {
            Self::Local(_) => Ok(()),
            Self::Ssh { .. } => {
                require("rsync", "rsync")?;
                require("ssh", "openssh-client")
            }
            Self::S3 { .. } => Ok(()),
        }
    }

    /// What to print, and what a `--from` would have to say to read it
    /// back.
    pub fn display(&self) -> String {
        match self {
            Self::Local(path) => path.display().to_string(),
            Self::Ssh { host, path } => format!("ssh://{host}{path}"),
            Self::S3 { bucket, prefix } if prefix.is_empty() => format!("s3://{bucket}"),
            Self::S3 { bucket, prefix } => format!("s3://{bucket}/{prefix}"),
        }
    }
}

/// Send a finished backup root to a remote destination.
///
/// `root` is the staging directory holding the whole shape —
/// `blobs/sha256/…` and `nodes/<id>/<taken at>/…` — so what arrives is
/// the same layout, and a shared root stays shared.
pub fn send(root: &Path, to: &Destination) -> Result<String, String> {
    match to {
        Destination::Local(_) => Ok("already there".into()),
        Destination::Ssh { host, path } => send_over_ssh(root, host, path),
        Destination::S3 { .. } => {
            Err("s3:// is not wired up yet — use ssh:// or a path for now".into())
        }
    }
}

/// Two transfers, because the two halves have different rules.
fn send_over_ssh(root: &Path, host: &str, path: &str) -> Result<String, String> {
    // The far side may be empty. rsync creates the last component of a
    // destination but not a chain of them, and the layout is two deep.
    run(
        "ssh",
        &[
            host,
            &format!("mkdir -p {}/blobs {}/nodes", quote(path), quote(path)),
        ],
    )?;

    let mut sent = Vec::new();

    // Blobs: skip what is there, whatever its size or age. A file under
    // a digest is the blob for that digest — see the module docs.
    let blobs = root.join("blobs");
    if blobs.is_dir() {
        let out = run(
            "rsync",
            &[
                "-a",
                "--ignore-existing",
                "--stats",
                &format!("{}/", blobs.display()),
                &format!("{host}:{}/blobs/", path),
            ],
        )?;
        sent.push(format!("blobs {}", transferred(&out)));
    }

    // Everything else is this run, and this run is new.
    let nodes = root.join("nodes");
    if nodes.is_dir() {
        let out = run(
            "rsync",
            &[
                "-a",
                "--stats",
                &format!("{}/", nodes.display()),
                &format!("{host}:{}/nodes/", path),
            ],
        )?;
        sent.push(format!("node {}", transferred(&out)));
    }

    Ok(sent.join(", "))
}

/// How many files rsync actually moved.
///
/// Read out of `--stats` rather than counted here, because the whole
/// point of `--ignore-existing` is that rsync decides what to skip and
/// this node does not know in advance. On the second machine of a
/// network sharing a root, the interesting number is how *few*.
fn transferred(output: &str) -> String {
    output
        .lines()
        .find_map(|line| line.strip_prefix("Number of regular files transferred: "))
        .map(|count| format!("{} sent", count.trim()))
        .unwrap_or_else(|| "sent".into())
}

/// A program this needs, and the package it comes in.
///
/// Checked here rather than added to `bootstrap::runtime::PROGRAMS`,
/// which `install` installs on every node: `rsync` and `ssh` are for
/// operators who chose a remote destination, and putting an SSH client
/// on every machine for a feature most will not use is a bigger
/// decision than this one. So it is a clear failure at the moment of
/// use, naming the package.
fn require(program: &str, package: &str) -> Result<(), String> {
    if Command::new(program).arg("--version").output().is_ok() {
        return Ok(());
    }
    Err(format!(
        "{program} is not on this machine, and a remote backup needs it. \
         Install {package} (`apk add {package}`, `apt install {package}`)."
    ))
}

fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| format!("could not run {program}: {error}"))?;
    if !output.status.success() {
        // Both streams, because rsync explains itself on stderr and ssh
        // sometimes on stdout, and the operator needs whichever it was.
        let detail = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr).trim(),
            String::from_utf8_lossy(&output.stdout).trim()
        );
        return Err(format!("{program} failed: {}", detail.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// A path as one word to the remote shell.
///
/// `ssh` hands its argument to a shell on the far side, so a directory
/// with a space in it becomes two arguments to `mkdir`. Single quotes,
/// with any single quote in the path closed and reopened around an
/// escaped one — the only sequence that is safe inside them.
fn quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_is_a_path() {
        assert_eq!(
            Destination::parse("/root/nb").expect("parsed"),
            Destination::Local(PathBuf::from("/root/nb"))
        );
        // Including one with a colon in it, which is the whole reason
        // the scp form is not accepted.
        assert_eq!(
            Destination::parse("/mnt/disk:1/backups").expect("parsed"),
            Destination::Local(PathBuf::from("/mnt/disk:1/backups"))
        );
    }

    #[test]
    fn ssh_names_a_host_and_an_absolute_directory() {
        assert_eq!(
            Destination::parse("ssh://root@backup.example/srv/wabot").expect("parsed"),
            Destination::Ssh {
                host: "root@backup.example".into(),
                path: "/srv/wabot".into()
            }
        );
        // A host with no directory is a mistake worth naming: rsync
        // would happily write into the login shell's home.
        assert!(Destination::parse("ssh://host").is_err());
        assert!(Destination::parse("ssh:///srv/x").is_err());
    }

    #[test]
    fn s3_takes_a_bucket_and_an_optional_prefix() {
        assert_eq!(
            Destination::parse("s3://my-bucket/nodes").expect("parsed"),
            Destination::S3 {
                bucket: "my-bucket".into(),
                prefix: "nodes".into()
            }
        );
        assert_eq!(
            Destination::parse("s3://my-bucket").expect("parsed"),
            Destination::S3 {
                bucket: "my-bucket".into(),
                prefix: String::new()
            }
        );
        // A trailing slash is somebody being tidy, not a prefix ending
        // in an empty segment.
        assert_eq!(
            Destination::parse("s3://my-bucket/nodes/").expect("parsed"),
            Destination::S3 {
                bucket: "my-bucket".into(),
                prefix: "nodes".into()
            }
        );
        assert!(Destination::parse("s3://").is_err());
    }

    /// A scheme this does not know must not become a directory called
    /// `sftp:` on the machine the operator was trying to get the backup
    /// *off*. That failure is silent until somebody needs the backup.
    #[test]
    fn an_unknown_scheme_is_refused_rather_than_treated_as_a_path() {
        for text in [
            "sftp://host/path",
            "s3:/bucket",
            "https://host/x",
            "s3:bucket",
        ] {
            let error = Destination::parse(text).expect_err(text);
            assert!(error.contains("not a destination"), "{text}: {error}");
        }

        // And a real path keeps its colons, which is the reason only the
        // first segment is examined.
        assert!(Destination::parse("/mnt/disk:1/backups").is_ok());
        assert!(Destination::parse("./backups:2026").is_ok());
    }

    /// Round-tripping matters: what is printed is what somebody will
    /// paste back into `--from`.
    #[test]
    fn what_is_printed_parses_back_to_the_same_place() {
        for text in [
            "/root/nb",
            "ssh://root@host/srv/wabot",
            "s3://bucket/prefix",
            "s3://bucket",
        ] {
            let parsed = Destination::parse(text).expect(text);
            assert_eq!(parsed.display(), text);
            assert_eq!(Destination::parse(&parsed.display()).expect(text), parsed);
        }
    }

    #[test]
    fn a_path_with_a_quote_stays_one_word() {
        assert_eq!(quote("/srv/it's here"), r#"'/srv/it'\''s here'"#);
    }

    /// rsync's own count is what gets reported, because
    /// `--ignore-existing` means rsync decides what to skip.
    #[test]
    fn the_count_comes_from_rsyncs_own_stats() {
        let stats = "Number of files: 12\nNumber of regular files transferred: 3\nTotal file size: 40 bytes\n";
        assert_eq!(transferred(stats), "3 sent");
        // And a version of rsync that words it differently must not
        // turn a successful transfer into a failure.
        assert_eq!(transferred("something else entirely"), "sent");
    }
}
