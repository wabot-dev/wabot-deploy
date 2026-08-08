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
    let explicit_domain = args.domain.is_some();
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

    let init = crate::bootstrap::init::Init::detect();
    if !args.no_system && init.supervises() {
        let unit_path = config_path.to_path_buf();
        let what = format!("the {} service", init.name());
        step(&database, Step::Service, &what, move || {
            let path = crate::bootstrap::service::unit_path();
            Ok(match crate::bootstrap::service::install_unit(&unit_path)? {
                true => format!("written to {}", path.display()),
                false => "already current".to_string(),
            })
        })
        .await?;
    } else if !args.no_system {
        println!("  no service manager here, so nothing was registered.");
        println!("  run `wabot-deploy serve` yourself, or supervise it however this machine does.");
    }

    report(&config, config_path, wrote_config, &database).await?;

    // What this node answers to, settled before the certificate is
    // asked for — that request reads it, and so does the console
    // afterwards.
    //
    // `--domain` is somebody saying it now, and wins. Without the flag
    // the stored value stands: a domain changed from the console must
    // survive the next `install`, which is what an upgrade runs.
    let previous = crate::node::settings::domain(&database, &config).await;
    let domain = match explicit_domain {
        true => config.node.domain.clone(),
        false => previous.clone(),
    };
    if let Some(domain) = &domain {
        crate::node::settings::set_domain(&database, Some(domain)).await?;
    }
    config.node.domain = domain.clone();
    let renamed = domain != previous;

    // Something to serve on the new name immediately, whatever the
    // authority says next. Without this a rename leaves the node
    // presenting a certificate for the name it stopped answering to.
    if renamed {
        if let Some(domain) = &domain {
            crate::edge::certs::ensure_self_signed(
                &database,
                crate::edge::certs::FALLBACK_NAME,
                &[
                    crate::edge::certs::FALLBACK_NAME.to_string(),
                    domain.clone(),
                ],
            )
            .await?;
        }
    }

    // A rename only takes effect once the edge is answering for the
    // new name: the console it serves is reached through the route
    // table, and the name that used to reach it must stop.
    if renamed {
        // Asked before anything is dropped: removing the old name
        // could empty the table, and "was this node already routing"
        // is the question — not "is it routing after the removal".
        let routing = !crate::edge::routes::load_all(&database).await?.is_empty();

        if let Some(previous) = &previous {
            crate::edge::routes::forget_control_plane(&database, previous).await?;
        }
        // Only into a table that already had rows. An empty one means
        // every hostname reaches the control plane — which is what
        // makes a fresh node reachable at its bare IP — and writing
        // the first row here would take that away.
        if let Some(domain) = &domain {
            if routing {
                crate::edge::routes::upsert(
                    &database,
                    domain,
                    &crate::edge::routes::Upstream::ControlPlane,
                    None,
                )
                .await?;
            }
        }
    }

    // Last, because it is the step that makes everything before it
    // true: the node comes up holding the ports, the certificate and
    // the configuration the earlier steps put in place.
    //
    // Not gated on the ledger, unlike every step above it. "Is the node
    // running the binary that is installed" is not a fact a previous
    // run can settle — it stops being true the moment somebody upgrades
    // the binary. A ledgered Start left the *old* process serving with
    // the new code sitting on disk beside it, and nothing said so.
    // Whether there is anything to wait for below. A node with a
    // certificate already in hand does not have to be disturbed; one
    // without may be deep in the renewal loop's backoff — up to six
    // hours — and a restart is what makes it try now.
    let wants_certificate = !config.acme.disabled
        && match &domain {
            Some(domain) => !has_public_certificate(&config, &database, domain).await,
            None => false,
        };

    if !args.no_system && !args.no_start && init.supervises() {
        use crate::bootstrap::service;

        // A rename restarts even when nothing else changed: the edge
        // reads its names and its routes at startup, so a node left
        // running is one still answering to the old domain.
        if service::is_active()
            && service::running_current_binary()
            && !renamed
            && !wants_certificate
        {
            println!();
            println!("  the node is already running this binary.");
        } else {
            println!("  the node…");
            ledger::record(&database, Step::Start, Status::Running, None).await?;
            match service::start() {
                Ok(()) => {
                    ledger::record(&database, Step::Start, Status::Done, Some("running".into()))
                        .await?;
                    println!();
                    println!(
                        "  the node is running. `{}`",
                        match init {
                            crate::bootstrap::init::Init::OpenRc =>
                                "rc-service wabot-deploy status",
                            _ => "systemctl status wabot-deploy",
                        }
                    );
                }
                Err(error) => {
                    let message = format!("{error:#}");
                    ledger::record(
                        &database,
                        Step::Start,
                        Status::Failed,
                        Some(message.clone()),
                    )
                    .await?;
                    println!("    failed: {message}");
                    return Err(error.into());
                }
            }
        }
    }

    // The certificate, last, because the node has to be *running* for
    // it to be possible at all: the HTTP-01 challenge response is
    // stored in the database and served over :80 by `serve`, so on a
    // machine where the node has never started there is nothing to
    // answer the authority and the order can only end Invalid.
    //
    // Asking before starting is what an earlier version did, and it
    // made a first install with a domain fail for ever: the failure
    // was fatal, so the node never started, so the next run failed the
    // same way.
    let node_running = init.supervises() && crate::bootstrap::service::is_active();
    let verdict = match (&domain, config.acme.disabled) {
        (Some(domain), false) => Some(
            await_certificate(
                &config,
                &database,
                domain,
                node_running,
                WAIT_FOR_CERTIFICATE,
            )
            .await,
        ),
        // No domain, or ACME turned off on purpose: nothing a public
        // authority could have been asked for.
        _ => None,
    };

    if let Some(verdict) = verdict {
        match verdict {
            Verdict::Obtained => {
                // A certificate in hand means whatever the last
                // attempt complained about is over. Leaving the
                // message would have the console reporting a failure
                // that has been fixed.
                crate::node::settings::set_acme_error(&database, None).await?;
            }
            Verdict::WillTryLater => {}
            Verdict::Failed if !args.allow_self_signed => {
                let reason = crate::node::settings::acme_error(&database)
                    .await
                    .unwrap_or_else(|| "no certificate arrived".into());
                println!();
                println!("  certificate: not obtained — {reason}");
                println!();
                println!("  The node is running and still retrying, on its own certificate.");
                println!();
                println!("  Most often DNS does not point here yet, or :80 is not reachable");
                println!("  from the internet, which is what the HTTP-01 challenge needs.");
                println!("  Fix that and run install again — or change the domain from the");
                println!("  node page, which checks DNS before it asks.");
                println!();
                println!("  To accept this node's own certificate instead:");
                println!("    wabot-deploy install --allow-self-signed");

                setup_token(&database, &config).await;
                database.close().await?;
                return Ok(1);
            }
            Verdict::Failed => {
                println!("  --allow-self-signed: serving this node's own certificate, and");
                println!("  retrying in the background. `wabot-deploy doctor` shows why.");
            }
        }
    }

    // Last of all, because it is the one thing here somebody has to
    // read and act on. A node with no administrator serves a console
    // nobody can get into, and this token is the way in.
    setup_token(&database, &config).await;

    database.close().await?;
    Ok(0)
}

/// What became of the certificate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Obtained,
    /// Nothing was running that could answer the challenge, so nothing
    /// was asked. Not a failure — the node requests one the first time
    /// it runs.
    WillTryLater,
    Failed,
}

/// Wait for the running node to obtain a certificate for `domain`.
///
/// Watching rather than ordering. `install` used to place the order
/// itself, with a resolver of its own — so a certificate it obtained
/// landed in the database but not in the running node's live resolver,
/// which kept serving the old one until its next pass. The node's
/// renewal loop runs as soon as it starts and installs what it gets;
/// this only has to look.
async fn await_certificate(
    config: &Config,
    database: &SqliteDatabase,
    domain: &str,
    node_running: bool,
    wait: std::time::Duration,
) -> Verdict {
    if has_public_certificate(config, database, domain).await {
        println!();
        println!(
            "  certificate: current, from {}",
            config.acme.directory_url()
        );
        return Verdict::Obtained;
    }

    if !node_running {
        println!();
        println!("  certificate: none yet for {domain}.");
        println!("  The node requests one the first time it runs — the challenge is");
        println!("  answered on :80 by the node itself, so it has to be up.");
        return Verdict::WillTryLater;
    }

    println!();
    println!("  requesting a certificate for {domain}…");
    let deadline = std::time::Instant::now() + wait;
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        if has_public_certificate(config, database, domain).await {
            println!(
                "  certificate obtained from {}",
                config.acme.directory_url()
            );
            if config.acme.is_staging() {
                println!("  (staging — browsers will not trust it; that is expected)");
            }
            return Verdict::Obtained;
        }
    }
    Verdict::Failed
}

/// Long enough for a DNS record that just propagated and an authority
/// under load; short enough that somebody watching a terminal does not
/// conclude it hung.
const WAIT_FOR_CERTIFICATE: std::time::Duration = std::time::Duration::from_secs(90);

/// Is there a live certificate for `domain` from the authority this
/// node was told to use?
///
/// The issuer is compared exactly against the directory URL, which is
/// the rule the renewal loop follows: `starts_with("acme")` once made
/// a staging certificate pass for a production one, and the node
/// served it for weeks.
async fn has_public_certificate(config: &Config, database: &SqliteDatabase, domain: &str) -> bool {
    match crate::edge::certs::load(database, domain).await {
        Ok(Some(stored)) => {
            stored.issuer == config.acme.directory_url()
                && stored.not_after > crate::platform::now_ms()
        }
        Ok(None) => false,
        Err(error) => {
            tracing::warn!(%error, "could not read the stored certificate");
            false
        }
    }
}

/// Issue the token that creates the first administrator.
///
/// Never fatal. The node is installed either way, and `wabot-deploy
/// setup-token` issues another — failing the whole install over a
/// token that can be re-minted would be the wrong trade.
async fn setup_token(database: &SqliteDatabase, config: &Config) {
    match crate::accounts::any_account(database).await {
        // Re-running install on a node somebody already set up must not
        // mint a token: that would be a way to take over a live node by
        // running the installer again.
        Ok(true) => {}
        // A token that is still good is not reissued. Re-running
        // install would otherwise invalidate the one the operator
        // copied out of the first run's output — converging by
        // breaking the thing it just handed them.
        Ok(false)
            if crate::accounts::setup_token_valid(database)
                .await
                .unwrap_or(false) =>
        {
            println!();
            println!("  a setup token from an earlier run is still valid.");
            println!("  `wabot-deploy setup-token` issues another if it is lost.");
        }
        Ok(false) => match crate::accounts::issue_setup_token(database).await {
            Ok(token) => crate::commands::setup_token::print(config, &token),
            Err(error) => {
                println!();
                println!("  could not issue a setup token: {error}");
                println!("  run `wabot-deploy setup-token` once the node is up.");
            }
        },
        Err(error) => {
            println!();
            println!("  could not check for an administrator: {error}");
        }
    }
}

/// Run one step and record how it went.
///
/// A failure is recorded and returned rather than swallowed: the point
/// of the ledger is that the next run knows where this one stopped.
///
/// **The ledger records; it does not gate.** Every step here is
/// convergent on its own — each asks the machine about the thing and
/// does nothing when the thing is already true — so skipping it
/// because a previous run said "done" only makes the answer stale.
/// That went wrong twice: a ledgered `Start` left the old binary
/// running after an upgrade, and a ledgered `Runtime` meant a node
/// that had containerd never got the CNI plugins added to that same
/// step later. Both times the install printed success and the node
/// was missing something.
async fn step<F>(database: &SqliteDatabase, which: Step, what: &str, work: F) -> anyhow::Result<()>
where
    F: FnOnce() -> anyhow::Result<String>,
{
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
    /// The install that used to be impossible.
    ///
    /// Nothing is running that could answer the HTTP-01 challenge —
    /// the response is served on :80 by the node itself — so there is
    /// nothing to fail about. An earlier version asked anyway, failed,
    /// and refused to start the node, which meant the next run failed
    /// the same way for ever.
    #[tokio::test]
    async fn an_install_that_cannot_start_the_node_does_not_fail_over_a_certificate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let mut config = config_in(dir.path());
        config.acme.disabled = false;
        config.acme.directory = "http://127.0.0.1:1/directory".into();

        let mut args = InstallArgs::with_domain("node.example.com");
        args.allow_self_signed = false;

        let code = run(config, &config_path, args).await.expect("install ran");
        assert_eq!(code, 0, "there was nothing that could have answered");
    }

    /// The four cases the exit code hangs on, in milliseconds instead
    /// of the minute and a half the real wait allows.
    #[tokio::test]
    async fn what_the_certificate_wait_concludes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = config_in(dir.path());
        config.acme.disabled = false;
        config.acme.directory = "https://acme-v02.api.letsencrypt.org/directory".into();
        let database = crate::db::open_in_memory().await.expect("open");
        let none = std::time::Duration::ZERO;

        // Nothing running, nothing stored: the node will ask when it
        // starts, and this run has nothing to report.
        assert_eq!(
            await_certificate(&config, &database, "node.example.com", false, none).await,
            Verdict::WillTryLater
        );

        // Running, and no certificate arrived within the window.
        assert_eq!(
            await_certificate(&config, &database, "node.example.com", true, none).await,
            Verdict::Failed
        );

        // A certificate from the authority we asked settles it, and
        // without waiting — this is the upgrade path, where the node
        // already has one.
        store_certificate(
            &database,
            "node.example.com",
            config.acme.directory_url(),
            crate::platform::now_ms() + 30 * 86_400_000,
        )
        .await;
        assert_eq!(
            await_certificate(&config, &database, "node.example.com", true, none).await,
            Verdict::Obtained
        );
    }

    /// The comparison that once cost weeks of a node serving a
    /// certificate no browser trusted: `starts_with("acme")` matched
    /// staging too. Nothing but the exact directory URL counts.
    #[tokio::test]
    async fn a_staging_certificate_is_not_the_one_that_was_asked_for() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = config_in(dir.path());
        config.acme.disabled = false;
        config.acme.directory = "production".into();
        let database = crate::db::open_in_memory().await.expect("open");

        store_certificate(
            &database,
            "node.example.com",
            "https://acme-staging-v02.api.letsencrypt.org/directory",
            crate::platform::now_ms() + 30 * 86_400_000,
        )
        .await;

        assert!(!has_public_certificate(&config, &database, "node.example.com").await);
    }

    /// An expired certificate is not a certificate. The node renews on
    /// its own, but an install that reported "current" over an expired
    /// row would be reporting the row, not the fact.
    #[tokio::test]
    async fn an_expired_certificate_does_not_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = config_in(dir.path());
        config.acme.disabled = false;
        let database = crate::db::open_in_memory().await.expect("open");

        store_certificate(
            &database,
            "node.example.com",
            config.acme.directory_url(),
            crate::platform::now_ms() - 1,
        )
        .await;

        assert!(!has_public_certificate(&config, &database, "node.example.com").await);
    }

    async fn store_certificate(
        database: &SqliteDatabase,
        domain: &str,
        issuer: &str,
        not_after: i64,
    ) {
        crate::edge::certs::save(
            database,
            &crate::edge::certs::StoredCert {
                domain: domain.to_string(),
                names: vec![domain.to_string()],
                cert_pem: "not a certificate".into(),
                key_pem: "not a key".into(),
                issuer: issuer.to_string(),
                not_after,
                source: crate::edge::certs::Source::parse(issuer),
            },
        )
        .await
        .expect("store");
    }

    /// ACME switched off is somebody saying up front that this node
    /// serves its own certificate. Failing that install would be
    /// refusing to do what was asked.
    #[tokio::test]
    async fn acme_disabled_is_not_a_failure() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");

        let mut args = InstallArgs::with_domain("node.example.com");
        args.allow_self_signed = false;

        let code = run(config_in(dir.path()), &config_path, args)
            .await
            .expect("install ran");
        assert_eq!(code, 0);
    }

    /// An upgrade re-runs `install`, and it must not undo a domain
    /// somebody changed from the console — the config file still holds
    /// whatever the first install wrote, and silently restoring it
    /// would point the node back at a name they stopped using.
    #[tokio::test]
    async fn a_domain_set_from_the_console_survives_an_upgrade() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let config = config_in(dir.path());
        let database_path = config.database_path();

        run(
            config.clone(),
            &config_path,
            InstallArgs::with_domain("first.example"),
        )
        .await
        .expect("install");

        // What the console does.
        let database = crate::db::open(&database_path).await.expect("open");
        crate::node::settings::set_domain(&database, Some("changed.example"))
            .await
            .expect("set");
        database.close().await.expect("close");

        // The upgrade: same config file, no --domain.
        let mut args = InstallArgs::none();
        args.domain = None;
        run(
            Config::load(&config_path).expect("load"),
            &config_path,
            args,
        )
        .await
        .expect("install");

        let database = crate::db::open(&database_path).await.expect("open");
        let stored = crate::node::settings::domain(&database, &config_in(dir.path())).await;
        assert_eq!(stored.as_deref(), Some("changed.example"));
    }

    /// Re-running with a different name is how somebody renames a
    /// node, and the rename has to reach the certificate and the route
    /// that make it answer — not just the setting.
    #[tokio::test]
    async fn a_second_install_with_another_domain_moves_the_node() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let config = config_in(dir.path());
        let database_path = config.database_path();

        run(
            config.clone(),
            &config_path,
            InstallArgs::with_domain("first.example"),
        )
        .await
        .expect("install");

        // A node past its first day: something has written routes.
        let database = crate::db::open(&database_path).await.expect("open");
        crate::edge::routes::upsert(
            &database,
            "first.example",
            &crate::edge::routes::Upstream::ControlPlane,
            None,
        )
        .await
        .expect("route");
        database.close().await.expect("close");

        run(
            Config::load(&config_path).expect("load"),
            &config_path,
            InstallArgs::with_domain("second.example"),
        )
        .await
        .expect("install");

        let database = crate::db::open(&database_path).await.expect("open");
        let hosts: Vec<String> = crate::edge::routes::load_all(&database)
            .await
            .expect("routes")
            .into_iter()
            .map(|(host, _)| host)
            .collect();
        assert!(hosts.contains(&"second.example".to_string()), "{hosts:?}");
        assert!(
            !hosts.contains(&"first.example".to_string()),
            "the old name still reaches the console: {hosts:?}"
        );

        // And a certificate exists for it, self-signed or not, so the
        // node is not presenting the old name's.
        let stored = crate::edge::certs::load(&database, crate::edge::certs::FALLBACK_NAME)
            .await
            .expect("load")
            .expect("a certificate");
        assert!(
            stored.names.iter().any(|name| name == "second.example"),
            "{:?}",
            stored.names
        );
    }

    /// A fresh node reaches its console at whatever address the
    /// operator can type — a bare IP, most often. Writing the first
    /// route here would end that.
    #[tokio::test]
    async fn the_first_install_writes_no_routes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let config = config_in(dir.path());
        let database_path = config.database_path();

        run(
            config,
            &config_path,
            InstallArgs::with_domain("first.example"),
        )
        .await
        .expect("install");

        let database = crate::db::open(&database_path).await.expect("open");
        assert!(crate::edge::routes::load_all(&database)
            .await
            .expect("routes")
            .is_empty());
    }

    /// …and `--domain` is somebody saying it now, which does win.
    #[tokio::test]
    async fn the_flag_overrides_what_was_stored() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let config = config_in(dir.path());
        let database_path = config.database_path();

        run(
            config.clone(),
            &config_path,
            InstallArgs::with_domain("first.example"),
        )
        .await
        .expect("install");

        let database = crate::db::open(&database_path).await.expect("open");
        crate::node::settings::set_domain(&database, Some("changed.example"))
            .await
            .expect("set");
        database.close().await.expect("close");

        run(
            Config::load(&config_path).expect("load"),
            &config_path,
            InstallArgs::with_domain("said.example"),
        )
        .await
        .expect("install");

        let database = crate::db::open(&database_path).await.expect("open");
        let stored = crate::node::settings::domain(&database, &config_in(dir.path())).await;
        assert_eq!(stored.as_deref(), Some("said.example"));
    }

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
