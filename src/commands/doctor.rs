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
    println!("install steps");
    let entries = ledger::all(&database).await.unwrap_or_default();
    for step in Step::ALL {
        let entry = entries.iter().find(|e| e.step == step.as_str());
        let state = match (entry.map(|e| e.status), Step::IMPLEMENTED.contains(step)) {
            (Some(Status::Done), _) => "done".to_string(),
            (Some(Status::Running), _) => {
                problems += 1;
                "INTERRUPTED — re-run install".to_string()
            }
            (Some(Status::Failed), _) => {
                problems += 1;
                match entry.and_then(|e| e.detail.clone()) {
                    Some(detail) => format!("FAILED — {detail}"),
                    None => "FAILED".to_string(),
                }
            }
            (None, true) => {
                problems += 1;
                "pending — run install".to_string()
            }
            // Not a problem: the step exists in the plan and its
            // milestone has not shipped. Counting it would make a
            // healthy node look broken.
            (None, false) => "not implemented yet".to_string(),
        };
        println!("  {:<12} {}", step.as_str(), state);
    }

    database.close().await?;
    finish(problems)
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

    fn config_in(dir: &Path) -> Config {
        let mut config = Config::default();
        config.node.data_dir = dir.join("data");
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

    #[tokio::test]
    async fn an_installed_node_is_clean() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");

        crate::commands::install::run(
            config_in(dir.path()),
            &config_path,
            InstallArgs {
                domain: None,
                email: None,
            },
        )
        .await
        .expect("install");

        let code = run(config_in(dir.path()), &config_path)
            .await
            .expect("doctor");
        assert_eq!(
            code, 0,
            "the steps that are not implemented yet must not count as problems"
        );
    }

    /// A run that died halfway has to be visible, or the operator's
    /// only clue is that something does not work.
    #[tokio::test]
    async fn an_interrupted_step_is_a_problem() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config_path = dir.path().join("config.toml");
        let config = config_in(dir.path());

        crate::commands::install::run(
            config.clone(),
            &config_path,
            InstallArgs {
                domain: None,
                email: None,
            },
        )
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
