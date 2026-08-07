//! Taking a release, on purpose, with one click.
//!
//! ## Never on its own
//!
//! Nothing here runs on a timer. A node that updates itself is a node
//! that restarts every service on it at a moment nobody chose — and
//! the whole product is "containers on a machine you own". The
//! console *offers*; a person decides.
//!
//! ## The order of the steps is the safety
//!
//! Download, checksum, run `--version`, back up the database, swap the
//! binary, restart. Each one is cheap to undo until the swap, and the
//! swap is a rename — atomic, with the previous binary kept beside it.
//! The database copy is taken before anything can migrate it, because
//! a migration is the one step that cannot be undone by putting the
//! old binary back.
//!
//! ## Who finishes the job
//!
//! The last step replaces this process, so it cannot report its own
//! outcome. The run is marked `restarting` and settled by the node
//! that comes back, which compares its own version against what the
//! row was going to — see [`settle_after_restart`].

pub mod github;
pub mod http;
pub mod notes;
pub mod runs;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use wabot::sqlite::SqliteDatabase;

use github::{Release, Version};
use runs::Status;

pub type UpdateResult<T> = Result<T, UpdateError>;

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error(transparent)]
    Releases(#[from] github::ReleaseError),
    #[error(transparent)]
    Http(#[from] http::HttpError),
    #[error(transparent)]
    Storage(#[from] wabot::sqlite::SqliteError),
    #[error("{0}")]
    Io(String),
    #[error("the download does not match the published checksum")]
    Checksum,
    #[error("the published checksum file is not one: {0:?}")]
    BadChecksumFile(String),
    #[error("the downloaded binary reports {got}, not {want}")]
    WrongVersion { got: String, want: String },
    #[error("the downloaded binary does not run here: {0}")]
    WillNotRun(String),
    #[error("an update to {0} is already running")]
    AlreadyRunning(String),
    #[error("{0} is what this node is already running")]
    AlreadyCurrent(String),
    #[error("{0} published no build this node can install")]
    NotInstallable(String),
    #[error("systemd is not managing this node, so it cannot restart itself")]
    NoSystemd,
}

/// The release list, and when it was read.
///
/// Cached because the page that shows it is one somebody refreshes,
/// and GitHub allows sixty unauthenticated requests an hour per
/// address. Fifteen minutes is well inside that with room for every
/// other thing on the machine that might talk to GitHub.
pub struct Catalogue {
    cached: Mutex<Option<(Instant, Vec<Release>)>>,
}

const CACHE_FOR: Duration = Duration::from_secs(15 * 60);

impl Default for Catalogue {
    fn default() -> Self {
        Self {
            cached: Mutex::new(None),
        }
    }
}

impl Catalogue {
    /// The releases, from cache when it is fresh.
    pub async fn releases(&self) -> UpdateResult<Vec<Release>> {
        let mut cached = self.cached.lock().await;
        if let Some((read_at, releases)) = cached.as_ref() {
            if read_at.elapsed() < CACHE_FOR {
                return Ok(releases.clone());
            }
        }

        let releases = github::releases().await?;
        *cached = Some((Instant::now(), releases.clone()));
        Ok(releases)
    }

    /// Forget what was read, so the next look asks GitHub.
    ///
    /// For the operator who just published a release and is staring at
    /// a page that does not show it — the alternative is explaining
    /// the cache.
    pub async fn refresh(&self) {
        *self.cached.lock().await = None;
    }
}

/// What the console shows: what is running, what is available, what
/// happened last time.
pub struct Availability {
    pub current: Option<Version>,
    pub releases: Vec<Release>,
    /// The newest installable release above the running version.
    pub upgrade: Option<Release>,
}

pub async fn availability(catalogue: &Catalogue) -> UpdateResult<Availability> {
    let releases = catalogue.releases().await?;
    let current = Version::current();
    let upgrade = github::newest_upgrade(&releases, current).cloned();
    Ok(Availability {
        current,
        releases,
        upgrade,
    })
}

/// Install a release, and restart into it.
///
/// Long-running: the caller starts it and lets go — see
/// [`start_in_background`]. Progress is on the run row.
pub async fn apply(
    database: Arc<SqliteDatabase>,
    config: crate::config::Config,
    catalogue: Arc<Catalogue>,
    tag: &str,
    account_id: Option<String>,
) -> UpdateResult<()> {
    let releases = catalogue.releases().await?;
    let release = github::find(&releases, tag)
        .ok_or_else(|| github::ReleaseError::Unknown(tag.to_string()))?
        .clone();

    if !release.installable() {
        return Err(UpdateError::NotInstallable(release.tag.clone()));
    }
    let current = Version::current();
    if current == Some(release.version) {
        return Err(UpdateError::AlreadyCurrent(release.tag.clone()));
    }
    if let Some(existing) = runs::in_flight(&database).await? {
        return Err(UpdateError::AlreadyRunning(existing.to_version));
    }
    // Refused before anything is downloaded rather than after: an
    // update that cannot restart the node ends with a new binary on
    // disk and the old one still serving, which is the confusing half
    // of a failure.
    if !crate::bootstrap::service::systemd_available() {
        return Err(UpdateError::NoSystemd);
    }

    let run = runs::start(
        &database,
        &current.map(|v| v.to_string()).unwrap_or_default(),
        &release.version.to_string(),
        &release.tag,
        account_id.as_deref(),
    )
    .await?;

    match install(&database, &config, &release, &run.id).await {
        Ok(()) => {
            // Nothing after this line is guaranteed to run: the
            // restart takes the process with it.
            runs::finish(
                &database,
                &run.id,
                Status::Restarting,
                Some("restarting into the new binary"),
            )
            .await?;
            restart()?;
            Ok(())
        }
        Err(error) => {
            let message = error.to_string();
            tracing::error!(%message, tag = %release.tag, "update failed");
            runs::finish(&database, &run.id, Status::Failed, Some(&message)).await?;
            Err(error)
        }
    }
}

/// Kick it off and answer the request now.
///
/// An update takes a minute of downloading on a slow line, and a
/// browser waiting on that would time out somewhere in the middle,
/// leaving the operator with no page and an update still running.
pub fn start_in_background(
    database: Arc<SqliteDatabase>,
    config: crate::config::Config,
    catalogue: Arc<Catalogue>,
    tag: String,
    account_id: Option<String>,
) {
    tokio::spawn(async move {
        if let Err(error) = apply(database, config, catalogue, &tag, account_id).await {
            tracing::error!(%error, %tag, "could not install the release");
        }
    });
}

async fn install(
    database: &SqliteDatabase,
    config: &crate::config::Config,
    release: &Release,
    run_id: &str,
) -> UpdateResult<()> {
    let binary = release.binary.as_ref().expect("installable");
    let checksum = release.checksum.as_ref().expect("installable");

    runs::set_step(database, run_id, "downloading the checksum").await?;
    let published = http::get_text(&checksum.url, "text/plain").await?;
    let expected = parse_checksum(&published)?;

    runs::set_step(database, run_id, "downloading the binary").await?;
    let bytes = http::get(&binary.url, "application/octet-stream").await?;

    runs::set_step(database, run_id, "checking what arrived").await?;
    if sha256_hex(&bytes) != expected {
        return Err(UpdateError::Checksum);
    }

    let target = installed_path();
    let staging = target.with_extension("incoming");
    write_executable(&staging, &bytes)?;

    // It ran here, and it is what it says it is. Two failures this
    // catches before the node restarts into them: a binary for the
    // wrong libc or architecture, and a release whose asset does not
    // match its tag.
    match version_of(&staging) {
        Ok(reported) if reported == release.version.to_string() => {}
        Ok(reported) => {
            let _ = std::fs::remove_file(&staging);
            return Err(UpdateError::WrongVersion {
                got: reported,
                want: release.version.to_string(),
            });
        }
        Err(error) => {
            let _ = std::fs::remove_file(&staging);
            return Err(error);
        }
    }

    // Before the swap, because the new binary migrates the schema when
    // it starts and a migration is the one step putting the old binary
    // back does not undo.
    runs::set_step(database, run_id, "backing up the database").await?;
    let backup = back_up(database, config, &release.version.to_string()).await?;
    runs::set_backup(database, run_id, &backup.to_string_lossy()).await?;

    runs::set_step(database, run_id, "installing").await?;
    swap(&staging, &target)?;
    Ok(())
}

/// `<sha256>  <filename>`, which is what `sha256sum` writes.
fn parse_checksum(published: &str) -> UpdateResult<String> {
    let first = published.lines().next().unwrap_or_default();
    let digest = first.split_whitespace().next().unwrap_or_default();
    let looks_right = digest.len() == 64 && digest.chars().all(|c| c.is_ascii_hexdigit());
    match looks_right {
        true => Ok(digest.to_ascii_lowercase()),
        false => Err(UpdateError::BadChecksumFile(
            first.chars().take(80).collect(),
        )),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Where the unit's `ExecStart` points.
///
/// Not `current_exe`: on a machine where somebody is running the
/// binary from a build directory, replacing *that* file would leave
/// the installed one — the one systemd starts — untouched.
fn installed_path() -> PathBuf {
    PathBuf::from(crate::bootstrap::service::BINARY_PATH)
}

fn write_executable(path: &Path, bytes: &[u8]) -> UpdateResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| UpdateError::Io(error.to_string()))?;
    }
    std::fs::write(path, bytes).map_err(|error| UpdateError::Io(error.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .map_err(|error| UpdateError::Io(error.to_string()))?;
    }
    Ok(())
}

/// Ask the downloaded file what it is.
fn version_of(path: &Path) -> UpdateResult<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .map_err(|error| UpdateError::WillNotRun(error.to_string()))?;

    if !output.status.success() {
        return Err(UpdateError::WillNotRun(format!(
            "exited with {}",
            output.status
        )));
    }
    // `clap` prints "wabot-deploy 0.2.0".
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .split_whitespace()
        .last()
        .unwrap_or_default()
        .to_string())
}

/// Put the new binary in place, keeping the old one beside it.
///
/// Renames, not copies: a copy onto a running binary fails with
/// ETXTBSY, and a rename is atomic — there is no moment where the path
/// holds half a program.
fn swap(staging: &Path, target: &Path) -> UpdateResult<()> {
    if target.exists() {
        let previous = target.with_extension("previous");
        // Kept for the operator who has to put things back by hand.
        // Not a rollback button: rolling back a schema is not a file
        // operation, which is what the database copy above is for.
        std::fs::rename(target, &previous).map_err(|error| UpdateError::Io(error.to_string()))?;
    }
    std::fs::rename(staging, target).map_err(|error| UpdateError::Io(error.to_string()))
}

/// Copy the database somewhere the new binary will not migrate.
///
/// `VACUUM INTO` rather than copying the file: SQLite is being written
/// to while this runs, and a byte copy of a live database is a copy of
/// a half-finished transaction.
async fn back_up(
    database: &SqliteDatabase,
    config: &crate::config::Config,
    version: &str,
) -> UpdateResult<PathBuf> {
    let directory = config.node.data_dir.join("backups");
    std::fs::create_dir_all(&directory).map_err(|error| UpdateError::Io(error.to_string()))?;

    let path = directory.join(format!("before-{version}-{}.db", crate::platform::now_ms()));
    // A leftover from an attempt that failed after this point would
    // make `VACUUM INTO` refuse — it will not write onto a file that
    // exists.
    let _ = std::fs::remove_file(&path);

    let destination = path.to_string_lossy().to_string();
    database
        .write(move |connection| {
            connection.execute("VACUUM INTO ?1", [destination])?;
            Ok(())
        })
        .await?;
    Ok(path)
}

/// Restart the node, from inside the node.
///
/// `systemd-run` rather than a plain `systemctl restart`: this process
/// is in the unit's control group, and stopping the unit kills
/// everything in it — including the `systemctl` that was asked to do
/// the restarting. A transient unit runs outside that group and
/// survives the stop.
fn restart() -> UpdateResult<()> {
    use crate::bootstrap::service::UNIT_NAME;

    let transient = std::process::Command::new("systemd-run")
        .args([
            "--collect",
            "--on-active=1",
            "--unit=wabot-deploy-selfupdate",
            "systemctl",
            "restart",
            UNIT_NAME,
        ])
        .status();

    match transient {
        Ok(status) if status.success() => Ok(()),
        other => {
            // Worth trying anyway: systemd queues the job when the
            // request arrives, and the request usually arrives before
            // the kill. It is the ordering that is not guaranteed,
            // which is why it is the fallback and not the plan.
            tracing::warn!(?other, "systemd-run did not take; restarting the plain way");
            std::process::Command::new("systemctl")
                .args(["restart", UNIT_NAME])
                .spawn()
                .map(|_| ())
                .map_err(|error| UpdateError::Io(error.to_string()))
        }
    }
}

/// Settle whatever the last process was in the middle of.
///
/// Called once at startup. A `restarting` row belongs to an update
/// that got as far as replacing the binary; whether it *worked* is
/// answered by what this process is, which is a thing only this
/// process can say.
pub async fn settle_after_restart(database: &SqliteDatabase) {
    let Ok(Some(run)) = runs::latest(database).await else {
        return;
    };
    if run.status != Status::Restarting {
        return;
    }

    let running = crate::api::VERSION;
    let (status, detail) = match running == run.to_version {
        true => (
            Status::Done,
            format!("running {running}, migrations applied at startup"),
        ),
        // The unit came back on something else: the swap did not take,
        // or systemd started the previous binary. Either way the
        // operator is running a version they did not ask for, and the
        // page has to say so rather than showing a success.
        false => (
            Status::Failed,
            format!("came back running {running}, not {}", run.to_version),
        ),
    };

    if let Err(error) = runs::finish(database, &run.id, status, Some(&detail)).await {
        tracing::warn!(%error, "could not settle the update record");
    } else {
        tracing::info!(status = status.as_str(), %detail, "update settled");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checksum_file_is_read_the_way_sha256sum_writes_it() {
        let digest = "a".repeat(64);
        assert_eq!(
            parse_checksum(&format!("{digest}  wabot-deploy-0.2.0-x86_64-linux\n"))
                .expect("parsed"),
            digest
        );
    }

    /// The bytes about to be executed are checked against this file.
    /// Anything that is not plainly a digest has to be refused, not
    /// coerced — an HTML error page starts with characters too.
    #[test]
    fn anything_else_is_refused() {
        for published in [
            "",
            "not a checksum",
            "<html><body>404</body></html>",
            &"z".repeat(64),
            &"a".repeat(63),
        ] {
            assert!(parse_checksum(published).is_err(), "{published:?}");
        }
    }

    #[test]
    fn the_digest_is_the_one_sha256sum_would_print() {
        // `printf 'abc' | sha256sum`
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[tokio::test]
    async fn the_backup_is_a_database_the_new_binary_can_open() {
        let directory = tempfile::tempdir().expect("tempdir");
        let mut config = crate::config::Config::default();
        config.node.data_dir = directory.path().to_path_buf();

        let database = crate::db::open(&directory.path().join("node.db"))
            .await
            .expect("open");
        crate::node::settings::set_domain(&database, Some("before.example"))
            .await
            .expect("set");

        let backup = back_up(&database, &config, "0.2.0").await.expect("backup");
        assert!(backup.exists());

        // Opened as a database, not compared as bytes: what matters is
        // that it can be restored, and `VACUUM INTO` writes a
        // different file than the original by design.
        let restored = crate::db::open(&backup).await.expect("reopen");
        assert_eq!(
            crate::node::settings::domain(&restored, &crate::config::Config::default())
                .await
                .as_deref(),
            Some("before.example")
        );
    }

    /// The row is the only thing that crosses the restart, so what the
    /// returning process makes of it is the whole report.
    #[tokio::test]
    async fn a_restart_into_the_expected_version_settles_as_done() {
        let database = crate::db::open_in_memory().await.expect("open");
        let run = runs::start(&database, "0.0.1", crate::api::VERSION, "vx", None)
            .await
            .expect("start");
        runs::finish(&database, &run.id, Status::Restarting, None)
            .await
            .expect("restarting");

        settle_after_restart(&database).await;

        let settled = runs::latest(&database).await.expect("read").expect("a run");
        assert_eq!(settled.status, Status::Done);
    }

    #[tokio::test]
    async fn a_restart_into_something_else_settles_as_failed() {
        let database = crate::db::open_in_memory().await.expect("open");
        let run = runs::start(&database, "0.0.1", "99.0.0", "v99.0.0", None)
            .await
            .expect("start");
        runs::finish(&database, &run.id, Status::Restarting, None)
            .await
            .expect("restarting");

        settle_after_restart(&database).await;

        let settled = runs::latest(&database).await.expect("read").expect("a run");
        assert_eq!(settled.status, Status::Failed);
        assert!(
            settled
                .detail
                .as_deref()
                .unwrap_or_default()
                .contains(crate::api::VERSION),
            "{settled:?}"
        );
    }

    /// A finished run is history. Settling it again on every boot
    /// would rewrite the record of what happened.
    #[tokio::test]
    async fn a_settled_run_is_left_alone() {
        let database = crate::db::open_in_memory().await.expect("open");
        let run = runs::start(&database, "0.0.1", "0.0.2", "v0.0.2", None)
            .await
            .expect("start");
        runs::finish(&database, &run.id, Status::Failed, Some("checksum"))
            .await
            .expect("failed");

        settle_after_restart(&database).await;

        let settled = runs::latest(&database).await.expect("read").expect("a run");
        assert_eq!(settled.status, Status::Failed);
        assert_eq!(settled.detail.as_deref(), Some("checksum"));
    }
}
