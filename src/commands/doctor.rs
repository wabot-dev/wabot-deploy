//! `wabot-deploy doctor` — what is set up, what is not.
//!
//! Read-only, and safe to run against a live node. The point is to
//! answer "why is this not working" without an operator having to
//! know where the database lives or how the ledger is spelled.

use std::path::Path;

use crate::config::Config;
use crate::ledger::{self, Status, Step};

pub async fn run(config: Config, config_path: &Path) -> anyhow::Result<i32> {
    let mut problems = 0usize;

    println!("wabot-deploy {}", crate::api::VERSION);
    println!();

    println!("configuration");
    println!(
        "  file      {}",
        describe(config_path.exists(), config_path)
    );
    println!(
        "  domain    {}",
        config
            .node
            .domain
            .clone()
            .unwrap_or_else(|| "(none — self-signed)".into())
    );
    println!(
        "  data      {}",
        describe(config.node.data_dir.exists(), &config.node.data_dir)
    );
    println!("  https     port {}", config.edge.https_port);
    println!("  http      port {}", config.edge.http_port);

    println!();
    println!("database");
    let database_path = config.database_path();
    if !database_path.exists() {
        println!(
            "  {}  MISSING — run `wabot-deploy install`",
            database_path.display()
        );
        problems += 1;
        return finish(problems);
    }
    println!("  {}", database_path.display());

    // Read-only in spirit, but opening applies pending migrations —
    // which is the honest thing: a schema the daemon would upgrade on
    // start is not a difference worth reporting as a problem.
    let database = match crate::db::open(&database_path).await {
        Ok(database) => database,
        Err(error) => {
            println!("  cannot open: {error}");
            return finish(problems + 1);
        }
    };

    match crate::db::status(&database).await {
        Ok(migrations) => {
            let drifted: Vec<&str> = migrations
                .iter()
                .filter(|m| m.drifted)
                .map(|m| m.id.as_str())
                .collect();
            println!(
                "  schema    {} migration(s) applied",
                migrations.iter().filter(|m| m.applied).count()
            );
            if !drifted.is_empty() {
                println!(
                    "  DRIFT     {} — the SQL changed after it ran",
                    drifted.join(", ")
                );
                problems += 1;
            }
        }
        Err(error) => {
            println!("  schema    unreadable: {error}");
            problems += 1;
        }
    }

    println!();
    println!("machine");
    // Ports are skipped when the node holds them: a running node
    // failing its own port check would be a healthy node reading as
    // broken.
    let node_running = crate::bootstrap::service::is_active();
    for check in crate::bootstrap::preflight::run(
        config.edge.https_port,
        config.edge.http_port,
        !node_running,
    ) {
        if check.blocks() {
            problems += 1;
        }
        println!("  {check}");
    }
    if node_running {
        println!("  ok    ports          held by the running node");
    }

    println!();
    println!("runtime");
    let runtime = crate::bootstrap::runtime::status();
    println!(
        "  containerd  {}",
        runtime
            .containerd
            .clone()
            .unwrap_or_else(|| "absent".into())
    );
    println!(
        "  crun        {}",
        runtime.crun.clone().unwrap_or_else(|| "absent".into())
    );
    println!(
        "  socket      {}",
        if runtime.socket {
            crate::bootstrap::runtime::SOCKET.to_string()
        } else {
            format!("{} (absent)", crate::bootstrap::runtime::SOCKET)
        }
    );
    let missing = crate::bootstrap::runtime::missing_programs();
    if !missing.is_empty() {
        // Not a containerd problem, and it looks like one: the
        // container starts and then fails to get an address.
        println!(
            "  missing     {} — no container can get a network",
            missing
                .iter()
                .map(|program| program.command)
                .collect::<Vec<_>>()
                .join(", ")
        );
        problems += 1;
    }
    if !runtime.ready() {
        // Not counted as a problem: nothing deploys containers yet, so
        // a node without containerd is incomplete rather than broken.
        println!("  (no containers can run until all three are present)");
    }

    println!();
    println!("service");
    let init = crate::bootstrap::init::Init::detect();
    if crate::bootstrap::service::supervised() {
        println!("  managed by  {}", init.name());
        println!(
            "  file        {}",
            crate::bootstrap::service::unit_path().display()
        );
        println!(
            "  state       {}",
            if node_running {
                "active"
            } else {
                "not running"
            }
        );
    } else {
        println!("  nothing supervises services here; the node has to be run in the foreground");
    }

    println!();
    println!("certificates");
    match crate::edge::certs::load_all(&database).await {
        Ok(certificates) if certificates.is_empty() => {
            println!("  none yet — `serve` issues one on first start");
        }
        Ok(certificates) => {
            for certificate in &certificates {
                let days = (certificate.not_after - now_ms()) / 86_400_000;
                let state = if days < 0 {
                    problems += 1;
                    "EXPIRED".to_string()
                } else if days < 15 {
                    // Not yet a problem: the renewal loop has a
                    // fortnight and several attempts. Worth saying,
                    // because if it is still here next week it is.
                    format!("{days}d left")
                } else {
                    format!("{days}d left")
                };
                println!(
                    "  {:<38} {:<14} {state}",
                    certificate.domain,
                    short_issuer(&certificate.issuer)
                );
                println!("    covers: {}", certificate.names.join(", "));
            }
        }
        Err(error) => {
            println!("  unreadable: {error}");
            problems += 1;
        }
    }

    // The reason a certificate is missing is recorded beside the
    // domain, so an operator sees it here rather than in the journal.
    let domain = crate::node::settings::domain(&database, &config).await;
    if let Some(error) = crate::node::settings::acme_error(&database).await {
        let name = domain.as_deref().unwrap_or("this node");
        println!("  last ACME failure for {name}:");
        println!("    {error}");
        problems += 1;
    }

    println!();
    println!("acme");
    if config.acme.disabled {
        println!("  disabled — the local authority's certificate is served");
    } else if domain.is_none() {
        println!("  no domain set; nothing a public authority could validate");
    } else {
        println!("  directory  {}", config.acme.directory_url());
        if config.acme.is_staging() {
            println!("  (staging — its certificates are untrusted by design)");
        }
        match &config.acme.email {
            Some(email) => println!("  contact    {email}"),
            None => println!("  contact    (none — the authority cannot warn you before expiry)"),
        }
    }

    println!();
    println!("install steps");
    let entries = ledger::all(&database).await.unwrap_or_default();
    for (step, state, is_problem) in step_states(&entries) {
        if is_problem {
            problems += 1;
        }
        println!("  {step:<12} {state}");
    }

    database.close().await?;
    finish(problems)
}

/// One line per step: what it is, how it reads, and whether it counts
/// against the node.
///
/// Pure, and separated from the printing for exactly one reason: the
/// decisions here — an interrupted step is a problem, an unshipped one
/// is not — are worth testing without a machine that happens to be
/// Linux and happens to be root.
fn step_states(entries: &[ledger::Entry]) -> Vec<(&'static str, String, bool)> {
    Step::ALL
        .iter()
        .map(|step| {
            let entry = entries.iter().find(|entry| entry.step == step.as_str());
            let (state, problem) = match (entry.map(|e| e.status), Step::IMPLEMENTED.contains(step))
            {
                (Some(Status::Done), _) => ("done".to_string(), false),
                (Some(Status::Running), _) => ("INTERRUPTED — re-run install".to_string(), true),
                (Some(Status::Failed), _) => (
                    match entry.and_then(|e| e.detail.clone()) {
                        Some(detail) => format!("FAILED — {detail}"),
                        None => "FAILED".to_string(),
                    },
                    true,
                ),
                (None, true) => ("pending — run install".to_string(), true),
                // The step exists in the plan and its milestone has
                // not shipped. Counting it would make a healthy node
                // look broken.
                (None, false) => ("not implemented yet".to_string(), false),
            };
            (step.as_str(), state, problem)
        })
        .collect()
}

/// A directory URL is what gets stored, and it is unreadable in a
/// column. The well-known ones get a name; anything else keeps its
/// host, which is the part that identifies it.
fn short_issuer(issuer: &str) -> String {
    match issuer {
        "self-signed" => "self-signed".to_string(),
        url if url.contains("acme-staging-v02.api.letsencrypt.org") => {
            "letsencrypt-staging".to_string()
        }
        url if url.contains("acme-v02.api.letsencrypt.org") => "letsencrypt".to_string(),
        url => url.split('/').nth(2).unwrap_or(url).to_string(),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

fn describe(exists: bool, path: &Path) -> String {
    if exists {
        path.display().to_string()
    } else {
        format!("{} (absent)", path.display())
    }
}

/// Exit code, because `doctor` is a thing a script runs: 0 when the
/// node is in the state it claims, 1 when a human needs to look.
fn finish(problems: usize) -> anyhow::Result<i32> {
    println!();
    if problems == 0 {
        println!("no problems found");
        Ok(0)
    } else {
        println!("{problems} problem(s) found");
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::InstallArgs;

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
    async fn an_uninstalled_node_reports_a_problem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let code = run(config_in(dir.path()), &dir.path().join("config.toml"))
            .await
            .expect("doctor");
        assert_eq!(code, 1, "no database means something to fix");
    }

    /// The decisions `doctor` makes about a step, tested without
    /// needing the machine underneath to be a Linux node running as
    /// root — which the preflight section correctly refuses on a
    /// developer's laptop.
    #[test]
    fn a_missing_implemented_step_is_a_problem_and_an_unshipped_one_is_not() {
        let states = step_states(&[]);

        let problems: Vec<&str> = states
            .iter()
            .filter(|(_, _, problem)| *problem)
            .map(|(step, _, _)| *step)
            .collect();
        let implemented: Vec<&str> = Step::IMPLEMENTED.iter().map(|step| step.as_str()).collect();
        assert_eq!(
            problems, implemented,
            "nothing run: exactly the shipped steps are missing"
        );

        // And the rest say so rather than reading as broken.
        assert!(states
            .iter()
            .filter(|(step, _, _)| !implemented.contains(step))
            .all(|(_, state, problem)| !problem && state.contains("not implemented")));
    }

    #[test]
    fn an_interrupted_step_outranks_a_missing_one() {
        let entries = vec![ledger::Entry {
            step: "database".into(),
            status: Status::Running,
            detail: None,
            updated_at: 0,
        }];
        let (_, state, problem) = step_states(&entries)
            .into_iter()
            .find(|(step, _, _)| *step == "database")
            .expect("database is a step");
        assert!(problem);
        assert!(state.contains("INTERRUPTED"), "{state}");
    }

    #[test]
    fn a_failed_step_carries_its_reason_into_the_report() {
        let entries = vec![ledger::Entry {
            step: "runtime".into(),
            status: Status::Failed,
            detail: Some("checksum mismatch".into()),
            updated_at: 0,
        }];
        let (_, state, problem) = step_states(&entries)
            .into_iter()
            .find(|(step, _, _)| *step == "runtime")
            .expect("runtime is a step");
        assert!(problem);
        assert!(state.contains("checksum mismatch"), "{state}");
    }

    /// A finished install has nothing to report about its steps.
    #[test]
    fn a_complete_ledger_is_clean() {
        let entries: Vec<ledger::Entry> = Step::IMPLEMENTED
            .iter()
            .map(|step| ledger::Entry {
                step: step.as_str().to_string(),
                status: Status::Done,
                detail: None,
                updated_at: 0,
            })
            .collect();
        assert!(
            step_states(&entries).iter().all(|(_, _, problem)| !problem),
            "a finished install is clean, and unshipped steps are not problems"
        );
    }

    /// A run that died halfway has to be visible, or the operator's
    /// only clue is that something does not work.
    #[tokio::test]
    async fn an_interrupted_step_is_a_problem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let config = config_in(dir.path());

        crate::commands::install::run(config.clone(), &config_path, InstallArgs::none())
            .await
            .expect("install");

        let database = crate::db::open(&config.database_path())
            .await
            .expect("open");
        ledger::record(&database, Step::Database, Status::Running, None)
            .await
            .expect("record");
        database.close().await.expect("close");

        assert_eq!(run(config, &config_path).await.expect("doctor"), 1);
    }
}
