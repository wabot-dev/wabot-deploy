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
    // The file's value, and said to be the file's value. What the node
    // answers to is the stored one — the file only seeds it — so a node
    // renamed from the console has a config file naming something it
    // stopped answering to years ago. Printing that as `domain` made
    // this report contradict its own `network` section a page later,
    // which was found on a node and is exactly the kind of thing that
    // can only be found there.
    println!(
        "  domain    {} (in the file)",
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

    // A memory ceiling is a cgroup, and a cgroup v2 tree with the
    // memory controller is what crun writes it into. Where there is
    // none the limit is *silently ignored* — the container starts,
    // the page says 128 MB, and the process takes the machine. That is
    // the failure this line exists to make visible, and it is one this
    // report can answer without asking containerd anything.
    println!("  cgroups     {}", cgroup_memory());
    if !cgroup_memory_works() {
        problems += 1;
    }

    println!();
    println!("storage");
    match live_containers(&database).await {
        Ok(live) => {
            // What is on the disk, not how many containers were asked
            // about. The first version printed the container count under
            // the word `volumes`, so a node with two containers and no
            // storage at all reported `volumes 2` — a report that says
            // something untrue is worse than one that says nothing.
            let root = crate::platform::volumes::root(&config.node.data_dir);
            let stored = std::fs::read_dir(&root)
                .map(|entries| entries.flatten().filter(|e| e.path().is_dir()).count())
                .unwrap_or(0);
            println!("  volumes     {stored} in {}", root.display());
            // Listed, never removed. A directory whose rows are missing
            // for a reason nobody has understood yet is one somebody can
            // still recover from — the same rule reconciliation follows
            // about a container no row claims. Not counted as a problem
            // for the same reason: it is disk to reclaim, by hand, once
            // somebody has looked.
            // All four kinds, not only the data. A copy leaves its
            // configuration, its names and its log beside its volume,
            // and for a long time this looked for one of the four — so
            // the two nobody could see accumulated a file per container
            // that ever ran here, silently.
            for (kind, orphan) in crate::deploy::Deployer::leftovers(&config.node.data_dir, &live) {
                println!(
                    "  orphan      {} ({kind}, no replica claims it)",
                    orphan.display()
                );
            }
        }
        Err(error) => println!("  volumes     unreadable: {error}"),
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

    // The way back from an update, which exists and was written down
    // nowhere. Both halves are kept deliberately — the binary that was
    // replaced, and a copy of the database taken before anything could
    // migrate it — and an operator who needs them at two in the morning
    // was expected to know two paths nobody had told them. There is no
    // button for this on purpose: putting a schema back is not a file
    // operation, which is exactly why the second half is a copy.
    let previous =
        std::path::Path::new(crate::bootstrap::service::BINARY_PATH).with_extension("previous");
    if previous.exists() {
        println!("  the way back {}", previous.display());
    }
    let backups = config.node.data_dir.join("backups");
    if let Some((path, count)) = newest_backup(&backups) {
        println!(
            "  database copy {} ({count} kept in {})",
            path.display(),
            backups.display()
        );
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
    println!("network");
    // The one place `join` can be checked from. Everything it wrote is
    // on this machine, and until there is a tunnel there is nothing to
    // ask the other node — so what an operator needs is this printed
    // back rather than a connection test that cannot exist yet.
    match crate::network::me(&database).await {
        Ok(Some(me)) => {
            println!("  this node   {} ({})", me.name, me.id);
            // Said only when the two disagree. Agreement is the usual
            // case and needs no line; disagreement is a node answering
            // to a name its own config file does not mention, which
            // reads as a mistake until somebody explains it.
            if let Some(stored) = &domain {
                if config.node.domain.as_deref() != Some(stored.as_str()) {
                    println!(
                        "              renamed since install — the file still says {}",
                        config.node.domain.as_deref().unwrap_or("nothing")
                    );
                }
            }
            println!("  kind        {}", me.kind.as_str());
            println!(
                "  reachable   {}",
                me.endpoint.as_deref().unwrap_or("(no address to dial)")
            );
            println!(
                "  overlay     {}",
                me.overlay_ip.as_deref().unwrap_or("(not on one)")
            );
            println!(
                "  public key  {}",
                me.public_key.as_deref().unwrap_or("(none yet)")
            );
        }
        Ok(None) => {
            // Convergent on the next start, so it is worth saying and
            // not worth failing over.
            println!("  this node has no row of its own yet; `serve` writes one");
        }
        Err(error) => {
            println!("  unreadable: {error}");
            problems += 1;
        }
    }

    match crate::network::authorities(&database).await {
        Ok(authorities) if authorities.is_empty() => {
            println!("  takes instructions from nobody");
        }
        Ok(authorities) => {
            for authority in &authorities {
                println!(
                    "  authority   {} {}",
                    authority.node_id,
                    if authority.live() {
                        "allowed"
                    } else {
                        "revoked"
                    }
                );
            }
        }
        Err(error) => {
            println!("  authorities unreadable: {error}");
            problems += 1;
        }
    }

    // What the kernel says, not what this process asked for. An
    // overlay reported from a struct filled in at startup would answer
    // "did I try", and the question is whether packets move.
    //
    // The port included. It was printed from the configuration file
    // under this very comment: the peers below were the kernel's and
    // the number above them was not, so a node listening on something
    // other than what its file says reported the file. Now the file is
    // named only where there is no interface to ask.
    match crate::network::tunnel::observed() {
        Ok(interface) if interface.peers.is_empty() => {
            println!(
                "  interface   {} on udp/{}",
                crate::network::tunnel::INTERFACE,
                interface.port
            );
            println!("  no peers configured on it");
        }
        Ok(interface) => {
            println!(
                "  interface   {} on udp/{}",
                crate::network::tunnel::INTERFACE,
                interface.port
            );
            for peer in &interface.peers {
                println!(
                    "  peer        {} {}",
                    peer.public_key,
                    match peer.last_handshake {
                        Some(at) => {
                            let ago = (now_ms() / 1000).saturating_sub(at as i64);
                            format!(
                                "handshake {ago}s ago, {} in / {} out",
                                peer.rx_bytes, peer.tx_bytes
                            )
                        }
                        // The failure an operator most needs to tell
                        // apart from working: configured, and never
                        // once heard from.
                        None => "NEVER SHAKEN HANDS".to_string(),
                    }
                );
                if let Some(endpoint) = &peer.endpoint {
                    println!("              at {endpoint}");
                }
            }
            if interface.peers.iter().any(|peer| !peer.live()) {
                problems += 1;
            }
        }
        Err(error) => {
            // Not counted: a node on no overlay has no interface, and
            // saying so is not the same as something being wrong. The
            // configured port is what there is to say here — it is what
            // this node would listen on, and nothing is listening.
            println!(
                "  interface   {} would be udp/{} (from the file)",
                crate::network::tunnel::INTERFACE,
                config.overlay.port
            );
            println!("  not up: {error}");
        }
    }

    match crate::network::all(&database).await {
        Ok(nodes) => {
            for node in nodes.iter().filter(|node| !node.is_self) {
                // The relationship, not the machine — the same rule the
                // console follows, and it was applied there and not
                // here. This node printed `private` for a machine whose
                // own `doctor`, two terminals away, said `public`. Both
                // were right; the pair was nonsense.
                println!(
                    "  knows       {} ({}) {}",
                    node.name,
                    node.id,
                    match &node.overlay_ip {
                        Some(address) => format!("on the overlay at {address}"),
                        None => "no address for it yet".to_string(),
                    }
                );

                // And whether this node can *reach* it — which the design
                // said was impossible until somebody measured it. A verified
                // call over the overlay is what makes the doorbell work and
                // what will make reading a remote copy's log possible, so
                // the state of that channel belongs where somebody looking
                // for a reason will find it.
                println!("              channel   {}", channel_to(node).await);
            }
        }
        Err(error) => {
            println!("  nodes unreadable: {error}");
            problems += 1;
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
/// Whether this node can reach that one over the overlay, verified.
///
/// Dialled by the name every node has and checked against the authority
/// that node presented — so a private node with no domain is verified too,
/// which is the whole of phase 9. The call is a plain `GET /`: what is
/// being tested is the handshake and the name, and any answer at all
/// proves both.
async fn channel_to(node: &crate::network::Node) -> String {
    if node.ca_pem.is_none() {
        return "not yet — it has not reported since this version".to_string();
    }
    let started = std::time::Instant::now();
    match crate::network::call::to_node(node, "/", None).await {
        Ok(status) => format!(
            "verified in {} ms (answered {})",
            started.elapsed().as_millis(),
            status.as_u16()
        ),
        Err(error) => format!("{error}"),
    }
}

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

/// The most recent database copy an update took, and how many there
/// are.
///
/// Counted rather than pruned: a copy of somebody's database is not a
/// thing this node deletes on its own, and the number is what tells an
/// operator whether the directory is worth looking at. `VACUUM INTO`
/// writes one per update and nothing has ever removed one.
fn newest_backup(directory: &std::path::Path) -> Option<(std::path::PathBuf, usize)> {
    let mut copies: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(directory)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|kind| kind == "db"))
        .filter_map(|entry| Some((entry.metadata().ok()?.modified().ok()?, entry.path())))
        .collect();
    copies.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
    let count = copies.len();
    copies.into_iter().next().map(|(_, path)| (path, count))
}

/// The container id of every copy this node runs.
///
/// What `volumes::orphans` compares the disk against, and the same
/// derivation the deploy path uses: project slug, service slug, slot.
/// Reading it any other way would let the two disagree, and the
/// disagreement would read as "this data belongs to nothing".
async fn live_containers(database: &wabot::sqlite::SqliteDatabase) -> anyhow::Result<Vec<String>> {
    crate::deploy::Deployer::claimed(database)
        .await
        .map(|claims| crate::deploy::Claim::containers(&claims))
        .ok_or_else(|| anyhow::anyhow!("could not read what this node claims"))
}

/// Whether a memory ceiling written into the spec will be honoured.
///
/// cgroup v2, mounted, with the `memory` controller available to a
/// child cgroup. All three matter and the third is the one that catches
/// people out: a v2 tree exists, and `memory` is missing from
/// `cgroup.subtree_control`, so crun writes `memory.max` into a
/// directory that has no such file.
///
/// Where this is false the limit is not refused — it is **ignored**.
/// The container starts, the page says 128 MB, and the process takes
/// the machine.
fn cgroup_memory_works() -> bool {
    // v2 is a single hierarchy at this path with this file in it. On a
    // v1 machine the path exists and the file does not.
    std::fs::read_to_string("/sys/fs/cgroup/cgroup.controllers")
        .map(|controllers| controllers.split_whitespace().any(|name| name == "memory"))
        .unwrap_or(false)
}

fn cgroup_memory() -> String {
    match std::fs::read_to_string("/sys/fs/cgroup/cgroup.controllers") {
        Ok(controllers) => match controllers.split_whitespace().any(|name| name == "memory") {
            true => "v2, memory controller available".to_string(),
            false => format!(
                "v2, but no memory controller ({}) — a memory ceiling would be ignored",
                controllers.trim()
            ),
        },
        Err(_) => "not cgroup v2 — a memory ceiling would be ignored".to_string(),
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
