//! `wabot-deploy install` — set the node up, converging.
//!
//! Three of the ten steps are implemented: the ones that need neither
//! root nor containerd, and that everything else depends on. The rest
//! are listed by `doctor` as pending rather than silently absent —
//! a plan you can see is worth more than one you have to infer.

use std::path::Path;

use wabot::sqlite::SqliteDatabase;

use crate::cli::InstallArgs;
use crate::config::Config;
use crate::ledger::{self, Status, Step};

pub async fn run(mut config: Config, config_path: &Path, args: InstallArgs) -> anyhow::Result<i32> {
    // Flags win over the file, and are then persisted — so
    // `install --domain x` once is the same as editing the file.
    if let Some(domain) = args.domain {
        config.node.domain = Some(domain);
    }
    if let Some(email) = args.email {
        config.acme.email = Some(email);
    }
    if args.acme_staging {
        config.acme.directory = "staging".into();
    }

    // Preflight before anything is written. A machine that cannot run
    // the node should be told so while nothing has changed on it.
    if !preflight(&config, args.skip_preflight)? {
        return Ok(1);
    }

    // Layout next, because the config file and the database both need
    // somewhere to live. Not ledgered until the database exists — the
    // ledger is *in* the database — so this step records itself last.
    let layout = layout(&config)?;
    let wrote_config = config.write_if_absent(config_path)?;

    let database = crate::db::open(&config.database_path()).await?;

    // These three run unconditionally — they are cheap and idempotent,
    // and two of them have to happen *before* there is a ledger to
    // consult, since the ledger lives in the database they create. The
    // recording is what is conditional: a step already done keeps the
    // timestamp of when it was done, so the ledger stays a record of
    // the install rather than of the last time install was typed.
    for (step, detail) in [
        (Step::Layout, format!("{layout} directories")),
        (
            Step::Config,
            if wrote_config {
                format!("written to {}", config_path.display())
            } else {
                format!("kept existing {}", config_path.display())
            },
        ),
        (Step::Database, config.database_path().display().to_string()),
    ] {
        if ledger::is_done(&database, step).await? {
            continue;
        }
        ledger::record(&database, step, Status::Done, Some(detail)).await?;
    }

    // The steps that change the machine rather than the data
    // directory. Each records itself, so a run that dies part-way
    // resumes rather than repeating what already worked.
    if args.no_system {
        println!("  --no-system: containerd, the binary, the unit and the start were skipped.");
    }

    if !args.no_runtime && !args.no_system {
        step(&database, Step::Runtime, "containerd and crun", || {
            crate::bootstrap::runtime::ensure().map_err(anyhow::Error::from)
        })
        .await?;
    }

    if !args.no_system {
        step(&database, Step::Binary, "the binary", || {
            Ok(match crate::bootstrap::service::install_binary()? {
                true => format!("installed to {}", crate::bootstrap::service::BINARY_PATH),
                false => "already current".to_string(),
            })
        })
        .await?;
    }

    if !args.no_system && crate::bootstrap::service::systemd_available() {
        let unit_path = config_path.to_path_buf();
        step(&database, Step::Service, "the systemd unit", move || {
            Ok(match crate::bootstrap::service::install_unit(&unit_path)? {
                true => format!("written to {}", crate::bootstrap::service::UNIT_PATH),
                false => "already current".to_string(),
            })
        })
        .await?;
    } else if !args.no_system {
        println!("  systemd is not here, so no service was registered.");
    }

    report(&config, config_path, wrote_config, &database).await?;

    // Attempted here so a DNS or firewall problem surfaces while the
    // operator is still looking at a terminal — but never fatal: the
    // node serves on the local authority's certificate either way, and
    // `serve` retries in the background. An install that failed
    // because a DNS record had not propagated yet would be a bad
    // reason to have no node.
    if let Err(error) = try_certificate(&config, &database).await {
        println!();
        println!("  certificate: not obtained yet — {error}");
        println!("  the node will serve its local certificate and keep retrying.");
    }

    // Last, because it is the step that makes everything before it
    // true: the node comes up holding the ports, the certificate and
    // the configuration the earlier steps put in place.
    if !args.no_system && !args.no_start && crate::bootstrap::service::systemd_available() {
        step(&database, Step::Start, "the node", || {
            crate::bootstrap::service::start()?;
            Ok("running".to_string())
        })
        .await?;
        println!();
        println!("  the node is running. `systemctl status wabot-deploy`");
    }

    database.close().await?;
    Ok(0)
}

/// Run one step, unless it is already done, and record how it went.
///
/// A failure is recorded and returned rather than swallowed: the point
/// of the ledger is that the next run knows where this one stopped.
async fn step<F>(database: &SqliteDatabase, which: Step, what: &str, work: F) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<String>,
{
    if ledger::is_done(database, which).await? {
        return Ok(());
    }

    println!("  {what}…");
    ledger::record(database, which, Status::Running, None).await?;

    match work() {
        Ok(detail) => {
            println!("    {detail}");
            ledger::record(database, which, Status::Done, Some(detail)).await?;
            Ok(())
        }
        Err(error) => {
            let message = format!("{error:#}");
            ledger::record(database, which, Status::Failed, Some(message.clone())).await?;
            println!("    failed: {message}");
            Err(error)
        }
    }
}

/// Check the machine, and say whether to continue.
fn preflight(config: &Config, skip: bool) -> anyhow::Result<bool> {
    use crate::bootstrap::preflight;

    if skip {
        println!("  preflight skipped (--skip-preflight)");
        return Ok(true);
    }

    // Ports are checked only when this install will start the node.
    // On a machine already running one, its own ports would read as
    // taken — a healthy node failing its own preflight.
    let check_ports = !crate::bootstrap::service::is_active();
    let checks = preflight::run(config.edge.https_port, config.edge.http_port, check_ports);

    println!("checking this machine");
    for check in &checks {
        println!("  {check}");
    }

    let blocking: Vec<&preflight::Check> = checks.iter().filter(|check| check.blocks()).collect();
    if blocking.is_empty() {
        println!();
        return Ok(true);
    }

    println!();
    println!(
        "  {} check(s) say this machine cannot run the node. Nothing has been changed.",
        blocking.len()
    );
    println!("  Fix them, or re-run with --skip-preflight if you know better.");
    Ok(false)
}

/// Create the directories, owner-only.
///
/// `0700` because the database will hold an admin token and the
/// certificate directory will hold private keys. Getting the mode
/// right at creation is easier than noticing later that it was 0755.
fn layout(config: &Config) -> anyhow::Result<usize> {
    let directories = [
        config.node.data_dir.clone(),
        config.node.data_dir.join("db"),
        config.certificates_dir(),
    ];

    for directory in &directories {
        std::fs::create_dir_all(directory)?;
        restrict(directory)?;
    }
    Ok(directories.len())
}

#[cfg(unix)]
fn restrict(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> std::io::Result<()> {
    // The product is a Linux daemon; this exists so the crate still
    // builds on a developer's other machine.
    Ok(())
}

async fn report(
    config: &Config,
    config_path: &Path,
    wrote_config: bool,
    database: &SqliteDatabase,
) -> anyhow::Result<()> {
    println!("wabot-deploy {} installed", crate::api::VERSION);
    println!();
    println!("  config    {}", config_path.display());
    if !wrote_config {
        println!("            (existing file kept)");
    }
    println!("  data      {}", config.node.data_dir.display());
    println!("  database  {}", config.database_path().display());
    match &config.node.domain {
        Some(domain) => println!("  domain    {domain}"),
        None => println!("  domain    (none — the node will serve a self-signed certificate)"),
    }

    // The certificate authority the node signs with until ACME. An
    // operator trusts this once; printing it here is what makes that
    // possible without them going looking in the database.
    println!();
    println!("  local certificate authority — trust this to reach the node without warnings:");
    println!(
        "  {}",
        crate::edge::certs::ca_certificate_path(config).display()
    );
    if let Err(error) = write_ca_bundle(config, database).await {
        println!("  (could not write it: {error})");
    }

    let pending: Vec<&str> = Step::ALL
        .iter()
        .filter(|step| !Step::IMPLEMENTED.contains(step))
        .map(|step| step.as_str())
        .collect();
    if !pending.is_empty() {
        println!();
        println!("  not yet implemented: {}", pending.join(", "));
        println!("  run `wabot-deploy doctor` to see the current state");
    }

    let _ = ledger::all(database).await?;
    Ok(())
}

/// Ask the certificate authority, once, synchronously.
async fn try_certificate(config: &Config, database: &SqliteDatabase) -> anyhow::Result<()> {
    if config.acme.disabled {
        return Ok(());
    }
    let Some(domain) = config.node.domain.clone() else {
        // Nothing a public authority could validate. Not a failure.
        return Ok(());
    };

    println!();
    println!("  requesting a certificate for {domain}…");
    let resolver = crate::edge::certs::CertResolver::new();
    match crate::edge::acme::ensure(database, config, &resolver).await? {
        true => {
            println!(
                "  certificate obtained from {}",
                config.acme.directory_url()
            );
            if config.acme.is_staging() {
                println!("  (staging — browsers will not trust it; that is expected)");
            }
        }
        false => println!("  the existing certificate is current"),
    }
    Ok(())
}

/// Export the CA to a file the operator can hand to a trust store.
///
/// Written rather than printed: a PEM block in terminal scrollback is
/// something to copy carefully, and a path is something to pass to
/// `security add-trusted-cert` or `update-ca-certificates`.
async fn write_ca_bundle(config: &Config, database: &SqliteDatabase) -> anyhow::Result<()> {
    let pem = crate::edge::certs::ca_certificate_pem(database).await?;
    let path = crate::edge::certs::ca_certificate_path(config);
    std::fs::write(&path, pem)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A node under a temporary directory, with ACME off.
    ///
    /// Off is not incidental: a test that reaches a certificate
    /// authority is slow, flaky, and — for a domain nobody here owns —
    /// spends somebody's rate limit to be told no. The ACME path is
    /// exercised against a real domain by hand, and by the unit tests
    /// in `edge::acme` that stop short of the network.
    fn config_in(dir: &Path) -> Config {
        let mut config = Config::default();
        config.node.data_dir = dir.join("data");
        config.acme.disabled = true;
        config
    }

    #[tokio::test]
    async fn install_creates_the_layout_config_and_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let config = config_in(dir.path());
        let database_path = config.database_path();

        let code = run(
            config,
            &config_path,
            InstallArgs::with_domain("node.example.com"),
        )
        .await
        .expect("install");

        assert_eq!(code, 0);
        assert!(config_path.exists(), "the config file was written");
        assert!(database_path.exists(), "the database was created");

        // The domain from the flag reached the file.
        let written = Config::load(&config_path).expect("load");
        assert_eq!(written.node.domain.as_deref(), Some("node.example.com"));
    }

    /// The property `install` is built around: running it again
    /// converges instead of repeating or failing.
    #[tokio::test]
    async fn install_is_idempotent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");

        for _ in 0..2 {
            run(config_in(dir.path()), &config_path, InstallArgs::none())
                .await
                .expect("install");
        }

        let database = crate::db::open(&config_in(dir.path()).database_path())
            .await
            .expect("open");
        // `--no-system` (what `InstallArgs::none` sets, so tests do
        // not write to /usr/local) runs the three steps that only
        // touch the data directory.
        let entries = ledger::all(&database).await.expect("ledger");
        assert_eq!(
            entries.len(),
            3,
            "one row per step, not one per run: {entries:?}"
        );
        assert!(entries.iter().all(|e| e.status == Status::Done));
    }

    /// An operator's edits survive a re-install — the flag is a way to
    /// set the value the first time, not to overwrite it every time.
    #[tokio::test]
    async fn a_second_install_does_not_clobber_the_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");

        run(
            config_in(dir.path()),
            &config_path,
            InstallArgs::with_domain("first.example.com"),
        )
        .await
        .expect("install");

        run(
            config_in(dir.path()),
            &config_path,
            InstallArgs::with_domain("second.example.com"),
        )
        .await
        .expect("install");

        assert_eq!(
            Config::load(&config_path)
                .expect("load")
                .node
                .domain
                .as_deref(),
            Some("first.example.com"),
            "the existing file wins; edit it or delete it"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_data_directories_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let config = config_in(dir.path());
        let data_dir = config.node.data_dir.clone();
        let certs = config.certificates_dir();

        run(config, &dir.path().join("config.toml"), InstallArgs::none())
            .await
            .expect("install");

        for path in [data_dir, certs] {
            let mode = std::fs::metadata(&path)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(
                mode & 0o777,
                0o700,
                "{} is {:o}; it will hold private keys",
                path.display(),
                mode & 0o777
            );
        }
    }
}
