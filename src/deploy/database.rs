//! What a database container needs beyond what a service's does.
//!
//! The arithmetic and the file contents are in `platform::postgres`,
//! which is pure and where the numbers can be argued with. This is the
//! half that touches the disk: it writes the two generated files and
//! says what to mount, what to pass and how much shared memory to give.
//!
//! ## Rewritten on every deployment
//!
//! Both files are built from the rows each time rather than kept. A
//! `pg_hba.conf` written when the database was created would not know
//! about the standby added this morning, and the failure for that is a
//! replication connection refused with a message about no matching
//! entry — which reads like a wrong password.

use std::path::{Path, PathBuf};

use crate::platform::databases::Database;
use crate::platform::{postgres, presets};
use crate::runtime::spec::BindMount;

/// Everything the deploy path adds to the request for this copy.
pub struct Preparation {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub mounts: Vec<BindMount>,
    pub shm_size: Option<u64>,
    /// Which copy this is. The deploy path reads it to decide whether
    /// anything has to happen *before* the container — a standby needs
    /// its volume seeded, a primary does not.
    pub role: postgres::Role,
    /// Where the primary answers, for the seed to dial. The same pair
    /// the conninfo is built from, kept apart because `pg_basebackup`
    /// takes a host and a port rather than a connection string.
    pub primary_endpoint: Option<(String, u16)>,
    /// The ceiling this copy is about to run at, which for a standby is
    /// also the thing its data directory has to be able to bear. Taken
    /// from here rather than recomputed by the caller: it is one of the
    /// arguments above, and two places deriving it is how they differ.
    pub max_connections: u32,
}

/// What this copy is, as the rows describe it.
pub struct Plan<'a> {
    pub data_dir: &'a Path,
    pub container_id: &'a str,
    pub database: &'a Database,
    pub role: postgres::Role,
    /// The ceiling the operator chose. A database always has one; the
    /// fallback is the smallest rung rather than "no limit", because an
    /// unbounded Postgres on a one-core node is the node.
    pub memory_limit: Option<u64>,
    /// The project's own `/24`, so containers beside it can connect.
    pub subnet: String,
    /// Every place a standby may dial in from, in CIDR form. One line
    /// each in `pg_hba.conf`.
    ///
    /// Not just the remote ones. A standby *here* arrives from the
    /// bridge gateway, because the `pg_basebackup` that seeds it runs in
    /// the host's network namespace — so the project's own subnet has to
    /// be in this list whenever a copy runs on this node. See
    /// `postgres::hba`, where the node taught this the hard way.
    pub replication_from: Vec<String>,
    /// Whether a port of the node reaches this, which decides whether
    /// `pg_hba.conf` admits a client from anywhere.
    pub published: bool,
    /// Where a standby dials its primary: the primary node's overlay
    /// address, or this node's bridge when both are here. `None` on the
    /// primary itself.
    ///
    /// A host and a port rather than a connection string, because the
    /// two things that need it want different shapes — the server takes
    /// `primary_conninfo`, `pg_basebackup` takes `--host --port` — and
    /// one of them building the other's is how they drift.
    pub primary: Option<(String, u16)>,
}

impl Plan<'_> {
    fn limit(&self) -> u64 {
        self.memory_limit.unwrap_or(presets::SMALLEST)
    }
}

/// Write what this copy reads, and say what it needs.
pub fn prepare(plan: &Plan<'_>) -> std::io::Result<Preparation> {
    let conf = config_dir(plan.data_dir, plan.container_id);
    let init = init_dir(plan.data_dir, plan.container_id);
    std::fs::create_dir_all(&conf)?;
    std::fs::create_dir_all(&init)?;

    std::fs::write(
        conf.join("pg_hba.conf"),
        postgres::hba(
            &plan.subnet,
            &plan.database.replication_user,
            &plan.replication_from,
            plan.published,
        ),
    )?;

    // Runs once, when the data directory is created, and is ignored
    // ever after — so writing it on every deployment costs a file and
    // means a database made before any of this existed would still get
    // its replication role if it were ever initialised again.
    std::fs::write(
        init.join("010-wabot-replication.sql"),
        postgres::init_script(
            &plan.database.replication_user,
            &plan.database.replication_password,
        ),
    )?;

    let args = match plan.role {
        postgres::Role::Primary => postgres::primary_arguments(plan.limit()),
        postgres::Role::Standby => postgres::standby_arguments(
            plan.limit(),
            // A standby with nowhere to dial would come up as a
            // primary of its own and start accepting writes into a
            // copy of somebody's data. An empty conninfo makes
            // Postgres refuse to start instead, which is the outcome
            // to want — and the caller refuses before reaching here.
            &plan
                .primary
                .as_ref()
                .map(|(host, port)| {
                    postgres::conninfo(
                        host,
                        *port,
                        &plan.database.replication_user,
                        &plan.database.replication_password,
                    )
                })
                .unwrap_or_default(),
            &postgres::slot_name(slot_of(plan.container_id)),
        ),
    };

    // The environment is the primary's alone. A standby's data
    // directory arrives from the base backup with the users already in
    // it, so the entrypoint finds `PG_VERSION` and never runs `initdb`.
    let env = match plan.role {
        postgres::Role::Primary => postgres::environment(
            &plan.database.admin_user,
            &plan.database.admin_password,
            &plan.database.database_name,
        )
        .into_iter()
        .collect(),
        // `PGDATA` still, because the entrypoint uses it to find the
        // directory it is not going to initialise.
        postgres::Role::Standby => {
            vec![("PGDATA".to_string(), postgres::PGDATA.to_string())]
        }
    };

    Ok(Preparation {
        args,
        env,
        mounts: vec![
            BindMount {
                source: conf,
                destination: postgres::CONFIG_MOUNT.to_string(),
                read_only: true,
            },
            BindMount {
                source: init,
                destination: postgres::INIT_MOUNT.to_string(),
                read_only: true,
            },
        ],
        shm_size: Some(presets::shm_for(plan.limit())),
        role: plan.role,
        primary_endpoint: plan.primary.clone(),
        max_connections: postgres::tuning(plan.limit()).max_connections,
    })
}

/// Where the server's certificate and key live on the node.
pub fn tls_dir(data_dir: &Path, container_id: &str) -> PathBuf {
    crate::platform::volumes::directory(data_dir, container_id, postgres::VOLUME).join("tls")
}

/// Put the certificate where the server will read it.
///
/// Written on every deployment from whatever the certificate store
/// holds for this name — so a renewal reaches the database by the same
/// path that put the first one there, and there is no second place a
/// certificate can be.
///
/// The key is `0600` here and the ownership is somebody else's problem:
/// see [`tls_owner_is_wrong`], because Postgres will not start unless it
/// can read it.
pub fn write_tls(
    data_dir: &Path,
    container_id: &str,
    cert_pem: &str,
    key_pem: &str,
) -> std::io::Result<PathBuf> {
    let dir = tls_dir(data_dir, container_id);
    std::fs::create_dir_all(&dir)?;

    // Whoever owns these already. Writing makes a file root's again, so
    // a renewal would hand the server a key it cannot read — the same
    // failure as the first placement, arriving weeks later when nobody
    // is looking. The first placement learns the user by asking the
    // image; every one after it reads the answer back off the disk.
    let owner = existing_owner(&dir);

    for (name, contents, mode) in [
        ("server.crt", cert_pem, 0o644),
        ("server.key", key_pem, 0o600),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, contents)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode))?;
            if let Some((uid, gid)) = owner {
                std::os::unix::fs::chown(&path, Some(uid), Some(gid))?;
            }
        }
        let _ = mode;
    }
    Ok(dir)
}

/// Who owns this copy's TLS directory, when somebody already does.
///
/// `None` before the first `chown`, and `None` for root — which is the
/// same answer, because root owning it is what the one-shot exists to
/// change.
#[cfg(unix)]
fn existing_owner(dir: &Path) -> Option<(u32, u32)> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(dir.join("server.key")).ok()?;
    match metadata.uid() {
        0 => None,
        uid => Some((uid, metadata.gid())),
    }
}

#[cfg(not(unix))]
fn existing_owner(_dir: &Path) -> Option<(u32, u32)> {
    None
}

/// Whether the key is still owned by whoever the node wrote it as.
///
/// Postgres reads its key as its own unprivileged user and refuses to
/// start when it cannot: *"private key file has wrong owner"*, or an
/// `EACCES` that reads like the file is missing. Every file this node
/// writes belongs to root.
///
/// Answering "which user" by guessing is how this breaks on the next
/// image: it is 70 on the alpine variant and 999 on the debian one. So
/// nothing here guesses — the *image* is asked, once, by a one-shot
/// container that runs `chown`, and after that the directory itself
/// records the answer. This returns whether that is still needed.
#[cfg(unix)]
pub fn tls_owner_is_wrong(data_dir: &Path, container_id: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    let key = tls_dir(data_dir, container_id).join("server.key");
    match std::fs::metadata(&key) {
        // Root still owns it, so the server cannot read it.
        Ok(metadata) => metadata.uid() == 0,
        // Nothing to own yet.
        Err(_) => false,
    }
}

#[cfg(not(unix))]
pub fn tls_owner_is_wrong(_data_dir: &Path, _container_id: &str) -> bool {
    false
}

/// Whether this copy's volume already holds a database.
///
/// The same question the image's entrypoint asks — is there a
/// `PG_VERSION` — so the node and the image cannot disagree about
/// whether a data directory needs initialising. "Is the directory
/// empty" is a different question with a different answer on any mount
/// that has a `lost+found` in it.
pub fn seeded(data_dir: &Path, container_id: &str) -> bool {
    crate::platform::volumes::directory(data_dir, container_id, postgres::VOLUME)
        .join("pgdata")
        .join(postgres::VERSION_FILE)
        .exists()
}

/// What a standby's data directory was last able to run at.
///
/// One number, `max_connections`, beside the volume rather than inside
/// `pgdata` — a base backup overwrites everything in there.
const CEILING_FILE: &str = ".wabot-max-connections";

/// Whether this volume can still be started at `max_connections`.
///
/// **The failure this exists for**: a standby refuses to start when
/// `max_connections` is lower than the primary's value *recorded in the
/// write-ahead log it has yet to replay* — not the primary's value now.
/// So lowering a database's memory takes both copies down a rung, the
/// primary restarts happily at the new one, and the standby aborts
/// recovery for ever:
///
/// ```text
/// FATAL: recovery aborted because of insufficient parameter settings
/// DETAIL: max_connections = 10 is a lower setting than on the primary
///         server, where its value was 40.
/// ```
///
/// It cannot get past the record that says so, and nothing it replays
/// afterwards can help, so the directory is finished. A fresh base
/// backup from the primary — which is already at the new rung — has no
/// such record in it.
///
/// Asked of the volume rather than worked out from what changed, which
/// is this node's rule everywhere else: the machine holding a copy may
/// have been switched off when the operator lowered the rung, and an
/// errand that arrives days later has to reach the same answer.
///
/// A volume with no record is one seeded before this was written. It is
/// left alone and given a record from here on — a full copy of every
/// standby on the node at the next deployment is a worse answer than
/// the one case somebody has already reported.
pub fn outgrown(data_dir: &Path, container_id: &str, max_connections: u32) -> bool {
    let Some(before) = recorded_ceiling(data_dir, container_id) else {
        return false;
    };
    before > max_connections
}

fn ceiling_path(data_dir: &Path, container_id: &str) -> PathBuf {
    crate::platform::volumes::directory(data_dir, container_id, postgres::VOLUME).join(CEILING_FILE)
}

fn recorded_ceiling(data_dir: &Path, container_id: &str) -> Option<u32> {
    std::fs::read_to_string(ceiling_path(data_dir, container_id))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Record what this copy is being started at, for the next deployment
/// to compare against.
pub fn record_ceiling(data_dir: &Path, container_id: &str, max_connections: u32) {
    let path = ceiling_path(data_dir, container_id);
    if let Err(error) = std::fs::write(&path, max_connections.to_string()) {
        // Not fatal: the worst it costs is the comparison above, and a
        // deployment that failed because of a note beside a volume
        // would be the wrong trade entirely.
        tracing::warn!(file = %path.display(), %error, "recording a copy's connection ceiling");
    }
}

/// Throw away a standby's data directory so the next deployment seeds
/// it again.
///
/// The volume itself stays, and so does everything beside `pgdata` in
/// it. **Only ever a standby**: it holds a copy of the primary and
/// nothing of its own, which is what makes this safe here and unsafe
/// one directory over.
pub fn discard_standby_data(data_dir: &Path, container_id: &str) -> std::io::Result<()> {
    let pgdata = crate::platform::volumes::directory(data_dir, container_id, postgres::VOLUME)
        .join("pgdata");
    match std::fs::remove_dir_all(&pgdata) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// Make a standby follow rather than accept writes.
///
/// Written from the rows on every deployment, not left behind by
/// `pg_basebackup -R`. The role is what the rows say it is; a file in
/// the volume is what it was the last time somebody looked.
pub fn write_standby_signal(data_dir: &Path, container_id: &str) -> std::io::Result<()> {
    let pgdata = crate::platform::volumes::directory(data_dir, container_id, postgres::VOLUME)
        .join("pgdata");
    std::fs::write(pgdata.join(postgres::STANDBY_SIGNAL), b"")
}

/// Throw away the generated files for one copy.
///
/// Unlike a volume, these hold nothing anybody would want back: they
/// are rebuilt from the rows at the next deployment. They still go with
/// the container rather than accumulating one directory per copy that
/// ever existed.
pub fn discard(data_dir: &Path, container_id: &str) {
    let path = data_dir.join("config").join(container_id);
    if let Err(error) = std::fs::remove_dir_all(&path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(directory = %path.display(), %error, "removing generated configuration");
        }
    }
}

/// Where this copy reads the files the node writes for it.
///
/// `pub(crate)` because the node's certificate authority goes in here too,
/// for **every** container and not only a database's: one destination, so a
/// managed engine gets it through the mount it already has rather than a
/// second bind on the same path.
pub(crate) fn config_dir(data_dir: &Path, container_id: &str) -> PathBuf {
    data_dir.join("config").join(container_id).join("conf")
}

fn init_dir(data_dir: &Path, container_id: &str) -> PathBuf {
    data_dir.join("config").join(container_id).join("initdb")
}

/// The slot a container id ends in.
///
/// `Replica::container_id` puts it there and leaves it off slot 1, so
/// the absence of a numeric last component *is* slot 1. Read back
/// rather than threaded through: the id is the one thing every layer
/// here already agrees about.
fn slot_of(container_id: &str) -> u32 {
    container_id
        .rsplit('.')
        .next()
        .and_then(|last| last.parse().ok())
        .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::databases::Engine;

    fn database() -> Database {
        Database {
            service_id: "svc-1".into(),
            engine: Engine::Postgres,
            version: "17".into(),
            admin_user: "orders".into(),
            admin_password: "secret".into(),
            database_name: "orders".into(),
            replication_user: "wabot_replication".into(),
            replication_password: "also-secret".into(),
            primary_slot: 1,
            primary_endpoint: None,
            owner_domain: None,
        }
    }

    fn plan<'a>(
        data_dir: &'a Path,
        container_id: &'a str,
        row: &'a Database,
        role: postgres::Role,
    ) -> Plan<'a> {
        Plan {
            data_dir,
            container_id,
            database: row,
            role,
            memory_limit: Some(128 * 1024 * 1024),
            subnet: "10.42.3.0/24".into(),
            replication_from: Vec::new(),
            published: false,
            primary: None,
        }
    }

    #[test]
    fn a_primary_gets_its_credentials_and_its_files() {
        let dir = tempfile::tempdir().expect("tempdir");
        let row = database();
        let prepared =
            prepare(&plan(dir.path(), "demo.db", &row, postgres::Role::Primary)).expect("prepared");

        let env: std::collections::BTreeMap<_, _> = prepared.env.into_iter().collect();
        assert_eq!(env.get("POSTGRES_USER").map(String::as_str), Some("orders"));
        assert_eq!(
            env.get("PGDATA").map(String::as_str),
            Some(postgres::PGDATA)
        );

        let conf = config_dir(dir.path(), "demo.db").join("pg_hba.conf");
        assert!(conf.exists());
        assert!(std::fs::read_to_string(&conf)
            .expect("read")
            .contains("10.42.3.0/24"));

        let script = init_dir(dir.path(), "demo.db").join("010-wabot-replication.sql");
        assert!(std::fs::read_to_string(script)
            .expect("read")
            .contains("wabot_replication"));

        assert_eq!(prepared.mounts.len(), 2);
        assert!(prepared.mounts.iter().all(|mount| mount.read_only));
        assert_eq!(prepared.shm_size, Some(32 * 1024 * 1024));
    }

    /// A standby's data directory arrives seeded, so the entrypoint
    /// finds `PG_VERSION` and never runs `initdb` — handing it
    /// `POSTGRES_PASSWORD` would describe an initialisation that is not
    /// going to happen.
    #[test]
    fn a_standby_is_told_where_to_follow_and_not_how_to_initialise() {
        let dir = tempfile::tempdir().expect("tempdir");
        let row = database();
        let mut plan = plan(dir.path(), "demo.db.2", &row, postgres::Role::Standby);
        plan.primary = Some(("10.42.0.1".into(), 30000));

        let prepared = prepare(&plan).expect("prepared");
        let env: std::collections::BTreeMap<_, _> = prepared.env.iter().cloned().collect();
        assert!(!env.contains_key("POSTGRES_PASSWORD"));
        assert_eq!(
            env.get("PGDATA").map(String::as_str),
            Some(postgres::PGDATA)
        );

        let arguments = prepared.args.join(" ");
        // The conninfo is built here, from the pair, so the server and
        // `pg_basebackup` cannot be told two different things.
        assert!(
            arguments.contains("primary_conninfo=host=10.42.0.1 port=30000"),
            "{arguments}"
        );
        assert!(arguments.contains("user=wabot_replication"), "{arguments}");
        // And the same pair comes back out for the seed to dial.
        assert_eq!(
            prepared.primary_endpoint,
            Some(("10.42.0.1".to_string(), 30000))
        );
        assert!(
            arguments.contains("primary_slot_name=wabot_slot_2"),
            "{arguments}"
        );
        assert!(arguments.contains("hot_standby=on"), "{arguments}");
    }

    /// A standby cannot be started below what its own log records.
    ///
    /// Postgres refuses recovery when `max_connections` is lower than
    /// the primary's value *in the log still to be replayed*, not the
    /// primary's value now — so lowering a database's memory took the
    /// primary down a rung happily and left the standby aborting
    /// recovery for ever, which is what Jorge found on the node with a
    /// 256 MB database moved to 64 MB.
    ///
    /// The number comes from the same `prepare` that writes the
    /// arguments, so the thing compared and the thing passed cannot
    /// differ.
    #[test]
    fn a_standby_whose_ceiling_came_down_has_to_be_copied_again() {
        let dir = tempfile::tempdir().expect("tempdir");
        let row = database();
        let id = "demo.db.2";

        let mut big = plan(dir.path(), id, &row, postgres::Role::Standby);
        big.memory_limit = Some(256 * 1024 * 1024);
        big.primary = Some(("10.42.0.1".into(), 30000));
        let big = prepare(&big).expect("prepared");
        assert_eq!(big.max_connections, 40);

        // Nothing recorded yet: a volume seeded before any of this
        // existed is left alone rather than copied again on sight.
        assert!(!outgrown(dir.path(), id, big.max_connections));
        crate::platform::volumes::ensure(dir.path(), id, postgres::VOLUME).expect("volume");
        record_ceiling(dir.path(), id, big.max_connections);

        let mut small = plan(dir.path(), id, &row, postgres::Role::Standby);
        small.memory_limit = Some(64 * 1024 * 1024);
        small.primary = Some(("10.42.0.1".into(), 30000));
        let small = prepare(&small).expect("prepared");
        assert_eq!(small.max_connections, 10);

        assert!(
            outgrown(dir.path(), id, small.max_connections),
            "the log records 40 and this would start at 10"
        );
        // The other direction is fine, and must not cost a copy: a
        // standby may run above the primary, never below it.
        assert!(!outgrown(dir.path(), id, big.max_connections));
        record_ceiling(dir.path(), id, small.max_connections);
        assert!(!outgrown(dir.path(), id, small.max_connections));

        // And what it throws away is the data directory alone — the
        // volume around it holds the certificate the server reads.
        let volume = crate::platform::volumes::directory(dir.path(), id, postgres::VOLUME);
        std::fs::create_dir_all(volume.join("pgdata")).expect("pgdata");
        std::fs::write(volume.join("pgdata").join(postgres::VERSION_FILE), "17").expect("write");
        std::fs::create_dir_all(tls_dir(dir.path(), id)).expect("tls");
        discard_standby_data(dir.path(), id).expect("discarded");
        assert!(!seeded(dir.path(), id), "the next deployment seeds it");
        assert!(tls_dir(dir.path(), id).exists(), "and the rest stays");
    }

    /// The slot is read back off the container id, which is the one
    /// thing every layer here already agrees about — and slot 1 is the
    /// one with no suffix.
    #[test]
    fn the_slot_comes_back_off_the_container_id() {
        assert_eq!(slot_of("demo.db"), 1);
        assert_eq!(slot_of("demo.db.2"), 2);
        assert_eq!(slot_of("demo.db.17"), 17);
        assert_eq!(slot_of("demo.api"), 1, "a name is not a slot");
    }

    /// Rebuilt from the rows every time: a file written when the
    /// database was created would not know about a standby added this
    /// morning.
    #[test]
    fn the_hba_is_rewritten_with_whatever_the_rows_say_now() {
        let dir = tempfile::tempdir().expect("tempdir");
        let row = database();
        prepare(&plan(dir.path(), "demo.db", &row, postgres::Role::Primary)).expect("first");

        let mut second = plan(dir.path(), "demo.db", &row, postgres::Role::Primary);
        second.replication_from = vec!["10.42.0.7/32".into()];
        prepare(&second).expect("second");

        let conf = std::fs::read_to_string(config_dir(dir.path(), "demo.db").join("pg_hba.conf"))
            .expect("read");
        assert!(conf.contains("10.42.0.7/32"), "{conf}");
    }

    #[test]
    fn discarding_what_is_not_there_is_quiet() {
        let dir = tempfile::tempdir().expect("tempdir");
        discard(dir.path(), "demo.db");

        let row = database();
        prepare(&plan(dir.path(), "demo.db", &row, postgres::Role::Primary)).expect("prepared");
        discard(dir.path(), "demo.db");
        assert!(!config_dir(dir.path(), "demo.db").exists());
    }
}
