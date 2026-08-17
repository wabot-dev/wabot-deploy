//! What a Postgres container is told, derived from its preset and its
//! role.
//!
//! Pure. Nothing here reads a row or touches a disk — it turns a size
//! and a role into arguments, an environment and two files, so the
//! numbers can be argued with in a test rather than on a node.
//!
//! ## The preset sets the engine's arithmetic, not just the cgroup
//!
//! A 64 MB ceiling with the stock configuration is a container that is
//! killed before it finishes starting: `shared_buffers` alone defaults
//! to 128 MB. Setting the limit without the settings is the mistake
//! this table exists to make impossible.
//!
//! ## Configuration arrives as arguments
//!
//! The image's entrypoint ends in `exec postgres "$@"`, so `-c
//! shared_buffers=32MB` reaches the server, is visible in `ctr
//! containers info`, and is recomputed on every deployment. Writing
//! `postgresql.conf` into the volume instead would put a second,
//! older, opinion inside the thing being configured.

use std::collections::BTreeMap;

/// Where the image comes from when nobody says otherwise.
///
/// Written out in full because containerd does not do Docker's
/// familiar-name normalisation: `postgres:17-alpine` names a *registry*
/// called `postgres`, and only the qualified form resolves.
///
/// Alpine for the size — around 80 MB against 150, on a node where a
/// build already takes twenty-five minutes. The cost is musl's
/// collation: a data directory seeded under one variant and opened
/// under the other wants a reindex. Nothing moves one, and this is the
/// note that says so before somebody does.
pub fn image_for(version: &str) -> String {
    format!("docker.io/library/postgres:{version}-alpine")
}

/// The major versions this node will start.
///
/// A list rather than "whatever the operator typed": the tag has to
/// exist on Docker Hub, and the failure for one that does not is a pull
/// error minutes later rather than a refusal in the form.
pub const VERSIONS: [&str; 4] = ["18", "17", "16", "15"];

/// The one it offers first.
pub const DEFAULT_VERSION: &str = "17";

/// The port Postgres listens on, and the one every client assumes.
pub const PORT: u16 = 5432;

/// Where the image keeps its data, and so where the volume is mounted.
///
/// `PGDATA` is a *subdirectory* of it — see [`PGDATA`].
pub const DATA_MOUNT: &str = "/var/lib/postgresql/data";

/// The data directory itself.
///
/// One level below the mount point, which is the documented workaround
/// for `initdb` refusing a bind-mounted directory: it wants to own the
/// directory it initialises, and a mount point belongs to the node.
pub const PGDATA: &str = "/var/lib/postgresql/data/pgdata";

/// Where the node's own generated files are mounted, read-only.
pub const CONFIG_MOUNT: &str = "/etc/wabot";

/// Where the server's certificate and key live.
///
/// **Inside the volume, not in the read-only config mount**, for one
/// reason: Postgres refuses to start unless the key is owned by its own
/// user or by root with the group able to read it, and the config mount
/// is root's alone. The volume is a directory the node can fix the
/// ownership of — see `deploy::database::own_tls`.
///
/// Beside `pgdata` rather than inside it, because `initdb` refuses a
/// data directory that is not empty and the files are written before the
/// first start.
pub const TLS_DIR: &str = "/var/lib/postgresql/data/tls";

/// The two files, as Postgres is told to read them.
pub fn certificate_path() -> String {
    format!("{TLS_DIR}/server.crt")
}

pub fn key_path() -> String {
    format!("{TLS_DIR}/server.key")
}

/// The files the image runs once, at `initdb`.
pub const INIT_MOUNT: &str = "/docker-entrypoint-initdb.d";

/// The name of the volume a database keeps its data in.
pub const VOLUME: &str = "data";

/// Which copy this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Accepts writes. One per database.
    Primary,
    /// Follows the primary and refuses them.
    Standby,
    /// Being restored: unpack a base backup, replay the log to a moment,
    /// and then be an ordinary primary of its own.
    ///
    /// Its own role rather than a flag on `Primary`, because what the
    /// deploy path does with an empty volume is completely different —
    /// `initdb` for one and an unpack for the other — and the two are
    /// indistinguishable from the volume, which is empty either way. An
    /// `initdb` where a restore was meant produces an empty database
    /// that starts, answers, and looks exactly like a restore that
    /// worked.
    Restoring,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Primary => "primary",
            Role::Standby => "standby",
            Role::Restoring => "restoring",
        }
    }
}

/// Everything derived from one preset.
///
/// Public so the console can show the numbers rather than only the
/// size: an operator choosing 128 MB deserves to see what their
/// database will actually be allowed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuning {
    pub shared_buffers_mb: u64,
    pub effective_cache_size_mb: u64,
    pub work_mem_mb: u64,
    pub maintenance_work_mem_mb: u64,
    pub max_connections: u32,
    pub max_wal_size_mb: u64,
    /// Workers per gather. Zero below 1 GB: each one is a process and a
    /// shared memory segment, and neither is free at these sizes.
    pub parallel_workers: u32,
}

const MB: u64 = 1024 * 1024;

/// The arithmetic, from the ceiling the operator chose.
///
/// Four of these are the usual rules of thumb and one is not:
/// `effective_cache_size` is **half** the limit rather than the usual
/// three quarters of the machine. Inside a cgroup the page cache is
/// charged to the cgroup, so the planner's idea of available cache
/// cannot exceed what the limit allows — and the limit is also holding
/// `shared_buffers`.
pub fn tuning(memory_limit: u64) -> Tuning {
    let mb = memory_limit / MB;
    Tuning {
        shared_buffers_mb: (mb / 4).max(8),
        effective_cache_size_mb: (mb / 2).max(16),
        // Per sort, per connection, and several at once in one query.
        // It grows far more slowly than the ceiling for that reason.
        work_mem_mb: match mb {
            0..=64 => 1,
            65..=256 => 2,
            257..=1024 => 4,
            1025..=2048 => 8,
            _ => 16,
        },
        maintenance_work_mem_mb: (mb / 8).clamp(8, 512),
        // A backend costs a few MB of its own before it does anything,
        // so this is a ceiling on how many can exist rather than a
        // target. The 64 MB rung says ten, and something has to be the
        // thing that says so.
        max_connections: match mb {
            0..=64 => 10,
            65..=128 => 20,
            129..=256 => 40,
            257..=512 => 60,
            513..=1024 => 100,
            1025..=2048 => 150,
            _ => 200,
        },
        // Disk rather than memory, and here anyway: the preset is the
        // "this is a small box" knob, and a 1 GB write-ahead log on a
        // node holding a 64 MB database is the same misjudgement one
        // layer down.
        max_wal_size_mb: match mb {
            0..=128 => 256,
            129..=512 => 512,
            513..=1024 => 1024,
            _ => 2048,
        },
        parallel_workers: match mb {
            0..=1023 => 0,
            _ => 2,
        },
    }
}

/// Where a primary drops its finished write-ahead log, inside the
/// container.
pub const ARCHIVE_MOUNT: &str = "/archive";

/// How often a primary is made to close a segment even if it is not
/// full, in seconds.
///
/// **This is the granularity of point-in-time recovery**, and the number
/// somebody is really choosing when they ask to go back to a minute. A
/// segment is 16 MB and an idle database would otherwise take hours to
/// fill one — so without this, "restore to 14:32" on a quiet afternoon
/// means restoring to whenever the last segment happened to close.
///
/// **It is a floor, not a promise.** The switch is made by the
/// checkpointer when it next wakes, so a segment closes *at least* this
/// long after the last one and in practice a little more — measured on
/// the node at two to three minutes for a value of sixty seconds. A
/// horizon shown to an operator has to be the time of the last archived
/// segment rather than "a minute ago", and this is why.
///
/// A minute costs one segment per minute per database, which sounds
/// expensive and is not: measured on the node, an idle 16 MB segment
/// gzips to 16 kB and a busy one to 54 kB, which is why the command
/// below compresses.
pub const ARCHIVE_TIMEOUT_SECONDS: u32 = 60;

/// What a primary runs to archive one finished segment.
///
/// ## `pg_receivewal` was the other way, and this is not it
///
/// A streaming receiver has a better recovery point — it loses only the
/// unflushed tail rather than up to one segment — and it costs a
/// long-lived container per database, for ever, on a node whose whole
/// premise is two processes. This is the primary's own work: a `cp` it
/// forks per segment, and no new process that has to be watched.
///
/// ## The failure mode is the opposite of a slot's, and worse
///
/// A replication slot that nobody drains is bounded by
/// `max_slot_wal_keep_size` — the slot breaks and the disk survives.
/// **Archiving has no such bound.** Postgres will not delete a segment
/// it has not archived, so a command that keeps failing fills the disk
/// and stops the database.
///
/// That decides the shape of everything downstream: this writes to a
/// bind mount on the node, which cannot fail for any reason a network
/// can. Getting it to object storage or another machine is a separate
/// step that may fail all it likes without touching the database.
///
/// ## `test !  -f` first
///
/// The archive command must never overwrite a segment it already has —
/// the documentation is unusually blunt about this, because a retry
/// after a partial write would replace good bytes with truncated ones,
/// and the copy that gets replaced is the one PITR needs. So: refuse if
/// it is there, and let Postgres retry.
pub fn archive_command() -> String {
    format!("test ! -f {ARCHIVE_MOUNT}/%f.gz && gzip -1 < %p > {ARCHIVE_MOUNT}/%f.gz.tmp && mv {ARCHIVE_MOUNT}/%f.gz.tmp {ARCHIVE_MOUNT}/%f.gz")
}

/// Where a backup being restored is mounted, inside the container.
pub const BASE_MOUNT: &str = "/base";

/// Unpack a base backup into a data directory, and leave it ready to
/// recover.
///
/// One shell command in the engine's own image, which is where `tar`,
/// `gunzip` and the `postgres` user all are — the same reasoning as
/// every other container this node runs against that image.
///
/// Four steps, and the order is the whole of it:
///
/// 1. **Refuse a directory that is not empty.** Untarring over a data
///    directory would mix two databases into something that starts and
///    is wrong, which is worse than either failing.
/// 2. `base.tar.gz` is the cluster; `pg_wal.tar.gz` is the log written
///    *during* the backup, which goes inside it. `pg_basebackup -Ft`
///    produces exactly these two.
/// 3. **`recovery.signal`**, whose presence is the instruction. Without
///    it the server opens the restored files as an ordinary database and
///    replays nothing — which looks like a successful restore to the
///    moment of the backup, and silently is not the moment that was
///    asked for.
/// 4. `chown`, because the node made the directory as root and the
///    server is not root. The same fault as the key and the archive,
///    caught here before it can be a third surprise.
pub fn unpack_base() -> Vec<String> {
    vec![
        "sh".into(),
        "-c".into(),
        format!(
            "set -e; \
             [ -z \"$(ls -A {PGDATA} 2>/dev/null)\" ] || {{ echo 'that data directory is not empty'; exit 1; }}; \
             mkdir -p {PGDATA}; \
             tar -xzf {BASE_MOUNT}/base.tar.gz -C {PGDATA}; \
             mkdir -p {PGDATA}/pg_wal; \
             tar -xzf {BASE_MOUNT}/pg_wal.tar.gz -C {PGDATA}/pg_wal; \
             touch {PGDATA}/{RECOVERY_SIGNAL}; \
             chown -R postgres:postgres {PGDATA}; \
             chmod 0700 {PGDATA}"
        ),
    ]
}

/// The file whose presence tells Postgres to replay rather than open.
///
/// Its *presence*, not its contents — an empty file is the whole
/// instruction. Postgres deletes it when recovery finishes, which is how
/// a restored database stops being one and becomes an ordinary primary.
pub const RECOVERY_SIGNAL: &str = "recovery.signal";

/// How a recovering server fetches an archived segment.
///
/// `%f` is the name it wants and `%p` where to put it. Reading from a
/// **read-only** mount of the original database's archive, which is the
/// arrangement that matters: a restore is a second database replaying
/// the first one's log, and a recovery that could write to that archive
/// could damage the thing it was restoring from.
///
/// `gunzip -c` because the archive command compressed. The pair has to
/// agree, and they are next to each other so that a change to one is
/// read beside the other.
pub fn restore_command() -> String {
    format!("gunzip -c {ARCHIVE_MOUNT}/%f.gz > %p")
}

/// What a server is started with to replay to a moment and stop there.
///
/// ## `promote`, not `pause`
///
/// The default when a target is reached is to **pause**, waiting for
/// somebody to say what happens next — which for a database being
/// restored into a console is a server that comes up refusing
/// connections and no obvious way to un-pause it. `promote` finishes
/// recovery and opens for writes: the restored copy is its own database
/// from that moment, which is what somebody asking for one wants.
///
/// It is also what makes this safe to do at all: the restored database
/// is a *new* one that took the original's past, and promoting it means
/// it can never be confused for the original or replay any further.
///
/// ## `exclusive` is deliberately not set
///
/// `recovery_target_inclusive` defaults to true: recovery stops *after*
/// the last transaction that committed at or before the target. Somebody
/// asking to go back to 14:32 means "the database as it was at 14:32",
/// which includes what committed in that second.
pub fn recovery_arguments(memory_limit: u64, target: Option<&str>) -> Vec<String> {
    let mut arguments = common_arguments(memory_limit);

    push(&mut arguments, "restore_command", &restore_command());
    push(&mut arguments, "recovery_target_action", "promote");
    // A moment, parsed by Postgres as a timestamp in the server's time
    // zone — which is UTC here, and why `console::layout::exactly`
    // labels every time it shows.
    //
    // No target is "as far as the log goes", which is what a node being
    // rebuilt wants: the last moment that was archived.
    if let Some(target) = target {
        push(&mut arguments, "recovery_target_time", target);
    }

    // **Never archives.** A restored database replaying somebody else's
    // log must not write into the archive it is reading from — and one
    // that has been promoted would start a new timeline and archive that
    // into the original's directory, where the original's own segments
    // are. Off, explicitly, so that a copy of the primary's arguments
    // cannot bring it back.
    push(&mut arguments, "archive_mode", "off");
    arguments
}

/// What a primary is started with.
pub fn primary_arguments(memory_limit: u64, archiving: bool) -> Vec<String> {
    let mut arguments = common_arguments(memory_limit);

    // Archiving, when the node has somewhere to put it.
    //
    // `archive_mode` is a postmaster setting, so this takes effect at
    // the next deployment rather than now — the same as a memory
    // ceiling, and for the same reason.
    //
    // Off is written explicitly rather than left out, so that turning
    // it off is a deployment that *turns it off* rather than one that
    // leaves whatever was there.
    match archiving {
        true => {
            push(&mut arguments, "archive_mode", "on");
            push(&mut arguments, "archive_command", &archive_command());
            push(
                &mut arguments,
                "archive_timeout",
                &format!("{ARCHIVE_TIMEOUT_SECONDS}s"),
            );
            // Enough detail in the log to recover *to* something: with
            // `minimal` the log holds what a crash needs and not what a
            // point in time does.
            push(&mut arguments, "wal_level", "replica");
        }
        false => push(&mut arguments, "archive_mode", "off"),
    }

    // A slot holds write-ahead log for a standby that is not there.
    // Unbounded, that fills the primary's disk when one never comes
    // back; without a slot at all, a standby that is away too long
    // breaks and has to be seeded again. The bound turns the second
    // failure into the first, and does it visibly.
    let tuning = tuning(memory_limit);
    push(
        &mut arguments,
        "max_slot_wal_keep_size",
        &format!("{}MB", tuning.max_wal_size_mb * 2),
    );
    arguments
}

/// What a standby is started with.
///
/// `primary_conninfo` is passed on every start rather than left in the
/// `postgresql.auto.conf` that `pg_basebackup -R` writes. That file is
/// a remembered fact inside the volume, and it goes stale the moment
/// anything about the primary changes; an argument is recomputed from
/// the rows every time, which is what everything else here does.
pub fn standby_arguments(memory_limit: u64, conninfo: &str, slot: &str) -> Vec<String> {
    let mut arguments = common_arguments(memory_limit);
    push(&mut arguments, "primary_conninfo", conninfo);
    push(&mut arguments, "primary_slot_name", slot);
    // Without this a standby applies the log and answers nothing, which
    // is a warm spare rather than a read replica.
    push(&mut arguments, "hot_standby", "on");
    arguments
}

fn common_arguments(memory_limit: u64) -> Vec<String> {
    let tuning = tuning(memory_limit);
    let mut arguments = Vec::new();

    // Passed explicitly rather than relied on. The entrypoint's own
    // temporary server appends `-c listen_addresses=''` *after* these,
    // so it still comes up on its socket alone while `initdb` runs.
    push(&mut arguments, "listen_addresses", "*");
    push(
        &mut arguments,
        "hba_file",
        &format!("{CONFIG_MOUNT}/pg_hba.conf"),
    );

    // TLS, in the server rather than in front of it.
    //
    // A terminator on the node would cover a *published* port and
    // nothing else: a container on the project's own bridge reaches
    // this container directly, and no proxy can stand between them. So
    // the only place "this database always requires encryption" can be
    // made true is inside the server — which is also the only place
    // `hostssl` means anything. See `hba`.
    push(&mut arguments, "ssl", "on");
    push(&mut arguments, "ssl_cert_file", &certificate_path());
    push(&mut arguments, "ssl_key_file", &key_path());

    push(
        &mut arguments,
        "shared_buffers",
        &format!("{}MB", tuning.shared_buffers_mb),
    );
    push(
        &mut arguments,
        "effective_cache_size",
        &format!("{}MB", tuning.effective_cache_size_mb),
    );
    push(
        &mut arguments,
        "work_mem",
        &format!("{}MB", tuning.work_mem_mb),
    );
    push(
        &mut arguments,
        "maintenance_work_mem",
        &format!("{}MB", tuning.maintenance_work_mem_mb),
    );
    push(
        &mut arguments,
        "max_connections",
        &tuning.max_connections.to_string(),
    );
    push(
        &mut arguments,
        "max_wal_size",
        &format!("{}MB", tuning.max_wal_size_mb),
    );
    push(
        &mut arguments,
        "max_parallel_workers_per_gather",
        &tuning.parallel_workers.to_string(),
    );
    arguments
}

fn push(arguments: &mut Vec<String>, name: &str, value: &str) {
    arguments.push("-c".to_string());
    arguments.push(format!("{name}={value}"));
}

/// The environment the image's entrypoint reads.
///
/// Only the primary needs it: a standby's data directory arrives from
/// the base backup with the users already in it, so the entrypoint
/// finds `PG_VERSION` and skips `initdb` entirely.
pub fn environment(
    admin_user: &str,
    admin_password: &str,
    database_name: &str,
) -> BTreeMap<String, String> {
    [
        ("POSTGRES_USER", admin_user),
        ("POSTGRES_PASSWORD", admin_password),
        ("POSTGRES_DB", database_name),
        ("PGDATA", PGDATA),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

/// Who may connect, from where, and how.
///
/// The node writes the whole file and passes `-c hba_file=`, because
/// the image's own line is `host all all all scram-sha-256` and **`all`
/// in the database column does not match a replication connection**.
/// With the stock file, a standby is refused with a message about no
/// matching entry — which reads like a password problem and is not.
///
/// Rebuilt on every deployment from the rows, so a standby added today
/// is allowed today rather than at the next `initdb`.
///
/// `bridge` is the project's own /24. `replication_from` is every place
/// a standby may dial in from, in CIDR form, and getting that list wrong
/// is the failure this function exists to prevent — twice now:
///
/// * **A copy on another node** arrives as that node's overlay address.
///   A packet forwarded from the port this node binds on its overlay is
///   not rewritten, so what Postgres sees is the far node's address.
/// * **A copy on *this* node** arrives from the **bridge gateway**, not
///   from a container. Its data directory is seeded by a
///   `pg_basebackup` that runs in the host's network namespace — it has
///   no bridge address of its own — so the primary sees `10.42.x.1`.
///   Found on a node: `no pg_hba.conf entry for replication connection
///   from host "10.42.2.1"`, six times, once per attempt.
///
/// The subnet line above covers neither. `all` in the database column
/// does not match a replication connection, which is the whole reason
/// these lines exist and is easy to forget a second time.
pub fn hba(
    bridge: &str,
    replication_user: &str,
    replication_from: &[String],
    published: bool,
) -> String {
    let mut file = String::from(
        "# Written by wabot-deploy on every deployment. Edits are lost.\n\
         #\n\
         # The socket, for the image's own entrypoint: `initdb` and the\n\
         # temporary server it starts run over it, and never finish\n\
         # without this line.\n\
         local   all           all                                     trust\n",
    );

    file.push_str(
        "\n# Everything else in this project. `hostssl`, never `host`: a\n\
         # database that would accept an unencrypted connection accepts\n\
         # one, and the only place that can be refused is here.\n",
    );
    file.push_str(&format!(
        "hostssl all           all             {bridge:<23} scram-sha-256\n"
    ));

    if published {
        file.push_str(
            "\n# This database is published on a port of the node, so a\n\
             # client may arrive from anywhere — and had better be\n\
             # encrypted.\n\
             hostssl all           all             0.0.0.0/0               scram-sha-256\n",
        );
    }

    if !replication_from.is_empty() {
        file.push_str(
            "\n# Read replicas: the overlay address of each node holding one,\n\
             # and this project's own subnet when one runs here — a local\n\
             # standby is seeded from the host's namespace, so it arrives\n\
             # from the bridge gateway. `all` above does not cover any of\n\
             # them: a replication connection names no database, and only\n\
             # `replication` matches it.\n",
        );
        for cidr in replication_from {
            file.push_str(&format!(
                "hostssl replication   {replication_user:<15} {cidr:<23} scram-sha-256\n"
            ));
        }
    }
    file
}

/// The script the image runs once, at `initdb`.
///
/// The replication role is created here rather than by connecting to a
/// running server, which would need a Postgres client in this binary
/// for one statement. Every database this node makes has the role from
/// birth, whether or not it ever gets a standby — so adding one later
/// needs no SQL at all.
///
/// The password is alphanumeric (`password::generate`), so there is
/// nothing in it a single-quoted SQL literal has to escape. The
/// assertion is in the tests rather than in a comment somebody can
/// stop believing.
pub fn init_script(replication_user: &str, replication_password: &str) -> String {
    format!(
        "-- Written by wabot-deploy. Runs once, when the data directory\n\
         -- is created, and never again.\n\
         --\n\
         -- Replication logs in as its own role: a node holding a read\n\
         -- replica has every byte of this database already, and the\n\
         -- one thing it must not have is the ability to write here.\n\
         CREATE ROLE \"{replication_user}\" WITH REPLICATION LOGIN PASSWORD '{replication_password}';\n"
    )
}

/// The command that copies a primary's data directory into an empty
/// volume.
///
/// Three things about it, and each is a decision:
///
/// * **`-C -S`** creates the replication slot on the primary as part of
///   the backup. Without a slot, a standby that is away long enough for
///   the primary to recycle its write-ahead log breaks and has to be
///   seeded again; the primary bounds how much it keeps with
///   `max_slot_wal_keep_size`, so the slot cannot fill its disk either.
/// * **`-X stream`** takes the log written *during* the backup on a
///   second connection. The default fetches it at the end, which fails
///   on a busy primary when the segment it needs has already gone.
/// * **No `-R`.** That would write `primary_conninfo` into the volume,
///   where it becomes a remembered fact that goes stale the moment
///   anything about the primary moves. The node passes it as an
///   argument on every start instead, and writes `standby.signal`
///   itself — see [`STANDBY_SIGNAL`].
///
/// ## The slot is dropped first, and that is not belt and braces
///
/// `-C` fails outright when the slot is already there — *"replication
/// slot "wabot_slot_2" already exists"* — and it is there whenever this
/// standby has lived before: a slot belongs to the primary, so removing
/// a copy on one machine leaves it behind on another, and nothing this
/// node can do to its own disk removes it. That is the second half of
/// the failure Jorge hit reseeding a standby; the first was the data
/// directory, which is `deploy::database`'s.
///
/// `DROP_REPLICATION_SLOT` is a *replication-protocol* command, so it
/// goes down the one connection the primary's `pg_hba.conf` already
/// admits from a standby — no SQL login, and nothing widened to allow
/// this. It is expected to fail for a standby being seeded for the
/// first time, which is why the shell runs the backup either way: what
/// matters is the slot being fresh, not who made it so.
///
/// The password travels in the environment (`PGPASSWORD`) rather than in
/// the command, because a command is in the container's spec on disk and
/// in `ctr containers info`.
pub fn base_backup(host: &str, port: u16, user: &str, slot: &str) -> Vec<String> {
    // Both tools read `PGPASSWORD`, and neither the slot name nor the
    // user is anything but `[A-Za-z0-9_]` — `slot_name` builds the
    // first from a number and `create` the second — so there is nothing
    // here a shell would read as anything but a word.
    let drop = format!(
        "psql \"host={host} port={port} user={user} dbname=postgres \
         replication=database sslmode=require\" \
         --no-psqlrc --quiet --command \"DROP_REPLICATION_SLOT {slot}\" || true"
    );
    let backup = [
        "pg_basebackup",
        "--host",
        host,
        "--port",
        &port.to_string(),
        "--username",
        user,
        "--pgdata",
        PGDATA,
        "--wal-method",
        "stream",
        "--create-slot",
        "--slot",
        slot,
        // Says what it is doing to stderr, which the node keeps. A base
        // backup that failed silently is a standby nobody can explain.
        "--progress",
        "--verbose",
        "--no-password",
    ]
    .join(" ");

    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("{drop}\n{backup}"),
    ]
}

/// The command that copies a running database into a backup.
///
/// A sibling of [`base_backup`] rather than a parameter on it, because
/// the flags differ for reasons worth reading side by side:
///
/// * **No slot.** Seeding creates a permanent one so the primary keeps
///   write-ahead log for a standby that will come back. A backup wants
///   the opposite — a temporary slot that exists for the length of the
///   copy and holds nothing afterwards. `-X stream` takes one on its
///   own, which is exactly the lifetime this needs.
/// * **`--format=tar --gzip`.** Seeding writes a data directory the
///   server will open, so it is plain. A backup is a thing that gets
///   *moved*, and the destination this is built for is object storage:
///   two objects rather than the several thousand small files a plain
///   copy makes. The cost is that a restore untars, which `restore`
///   does.
/// * **`--checkpoint=fast`.** Seeding can wait for the next natural
///   checkpoint; a backup somebody is standing in front of should not.
///   It costs the primary a burst of writes, which is the trade an
///   operator taking a backup has already agreed to.
///
/// `-X stream` on both, and for the same reason: it takes the log
/// written *during* the copy on a second connection, where the default
/// fetches it at the end and fails on a busy server when the segment it
/// needs has already gone.
pub fn base_backup_into(host: &str, port: u16, user: &str) -> Vec<String> {
    [
        "pg_basebackup",
        "--host",
        host,
        "--port",
        &port.to_string(),
        "--username",
        user,
        "--pgdata",
        BACKUP_MOUNT,
        "--format=tar",
        "--gzip",
        "--wal-method",
        "stream",
        "--checkpoint=fast",
        "--progress",
        "--verbose",
        "--no-password",
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect()
}

/// Where the backup container writes, inside itself.
pub const BACKUP_MOUNT: &str = "/backup";

/// The file that makes Postgres come up following somebody instead of
/// accepting writes.
///
/// Written by the node, on every deployment of a standby, because the
/// role is a fact about the rows rather than about what is in the
/// volume. Postgres deletes it when a standby is promoted — so a node
/// that recreated it after a promotion would un-promote a primary, which
/// is why promotion has to move `database.primary_slot` and not just
/// touch the data directory.
pub const STANDBY_SIGNAL: &str = "standby.signal";

/// Whether a data directory has a database in it already.
///
/// `PG_VERSION` is what the image's own entrypoint looks for to decide
/// whether to run `initdb`, so a standby's seed asks the same question
/// the same way. Anything else — a directory that exists, a directory
/// that is not empty — answers a different question: `lost+found` on a
/// fresh mount is not a database.
pub const VERSION_FILE: &str = "PG_VERSION";

/// How a standby dials its primary.
pub fn conninfo(host: &str, port: u16, user: &str, password: &str) -> String {
    // `application_name` is what shows up in `pg_stat_replication` on
    // the primary, which is the one place somebody looks to find out
    // whether a standby is following.
    //
    // **`require`, and it used to say `disable`.** This was written
    // before the server did its own TLS, and turning `hostssl` on for
    // every line of `pg_hba.conf` broke replication the same afternoon
    // — silently, because a standby that cannot connect is a container
    // that stays up and says nothing. The primary's log had it:
    // `no pg_hba.conf entry for replication connection … no
    // encryption`, once every five seconds, from both standbys.
    //
    // Not `verify-full`, which is the honest gap: that checks the name
    // against the certificate, and a standby dials an *address* — the
    // primary's on the bridge, or its node's on the overlay, and no
    // name resolves to the second one yet. Naming that endpoint is what
    // would let this be `verify-full`, and until then the encryption is
    // real and the identity is whatever the network gives it: a private
    // bridge, or WireGuard.
    format!(
        "host={host} port={port} user={user} password={password} \
         application_name=wabot sslmode=require"
    )
}

/// The command that asks a primary about its slots.
///
/// **The image's own `psql`, not a client in this binary.** Measured on
/// the node: 134 ms for the whole container lifecycle — create, start,
/// query, tear down — against 21 new crates for the alternative, fifteen
/// of which exist only to authenticate SCRAM-SHA-256. Three things
/// decided it, and the first is that this node already does exactly
/// this: `base_backup` runs the same image with a different command, and
/// the traps are already written down. The second is that the client
/// version then always matches the server's, where a pinned crate and a
/// Postgres 18 age apart. The third is the promise in
/// `docs/databases.md` that a second engine is a table of numbers and
/// two strings — a Postgres client in the binary helps MySQL not at all,
/// and MySQL's client comes in MySQL's image.
///
/// `-tAF'|'`: tuples only, unaligned, one separator. That is what those
/// flags are for, and it is the difference between parsing a table meant
/// for a person and reading three fields.
///
/// The password travels in the environment, like the base backup's and
/// for the same reason: a command is in the container's spec on disk and
/// in `ctr containers info`.
pub fn ask_slots(host: &str, port: u16, user: &str) -> Vec<String> {
    [
        "psql",
        "--host",
        host,
        "--port",
        &port.to_string(),
        "--username",
        user,
        "--dbname",
        "postgres",
        "--no-password",
        "--tuples-only",
        "--no-align",
        "--field-separator",
        "|",
        "--command",
        replication_query(),
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect()
}

/// What the primary says about one standby's slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotState {
    /// The slot number, read back out of `wabot_slot_N` — which is the
    /// replica's slot, one thing across the whole network, so this reads
    /// against the placement page with nothing to map between.
    pub slot: u32,
    /// Whether a standby is connected to it right now.
    ///
    /// **This is the signal.** A slot exists because a standby was
    /// seeded against it; inactive means that standby stopped following
    /// and nothing has noticed. It is not the same as the container
    /// being down — a copy can be up, healthy, and silently no longer
    /// replicating, which is the failure this whole thing is about.
    pub active: bool,
    /// How much write-ahead log the primary is holding for it, in bytes.
    ///
    /// The consequence, not the symptom: an inactive slot makes the
    /// primary keep WAL until `max_slot_wal_keep_size`, and this is the
    /// number that says how close that is. A standby that is merely
    /// behind shows a small one and needs nobody.
    pub held_bytes: i64,
}

/// What to ask the primary about its standbys.
///
/// `pg_replication_slots` rather than `pg_stat_replication`, and the
/// difference is the whole point: the second lists standbys that are
/// *connected*, so one that stopped following simply is not there —
/// absence as a signal, which cannot be told from a standby nobody ever
/// created. The slot is a row that exists either way and says `active`.
///
/// Tuples only, unaligned, pipe-separated: `psql -tAF'|'`. That is what
/// those flags are for, and it is the difference between parsing a
/// table meant for a person and reading three fields.
pub fn replication_query() -> &'static str {
    "SELECT slot_name, active, \
     COALESCE(pg_wal_lsn_diff(pg_current_wal_lsn(), restart_lsn)::int8, 0) \
     FROM pg_replication_slots WHERE slot_type = 'physical' ORDER BY slot_name"
}

/// Read what [`replication_query`] returned.
///
/// Tolerant on purpose. This parses the output of a program run in a
/// container, and the ways that arrives malformed — a notice on stdout,
/// a truncated read, a slot somebody made by hand — are not worth a
/// failure that hides the rows that *did* parse. A line that makes no
/// sense is skipped; the ones that do are returned.
pub fn parse_replication(text: &str) -> Vec<SlotState> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.trim().split('|');
            let name = fields.next()?;
            let slot = name.strip_prefix("wabot_slot_")?.parse().ok()?;
            let active = matches!(fields.next()?, "t" | "true");
            // A missing or unreadable byte count is nought rather than a
            // dropped row: `active` is the signal and this is the
            // detail, so losing the detail must not lose the signal.
            let held_bytes = fields.next().and_then(|n| n.parse().ok()).unwrap_or(0);
            Some(SlotState {
                slot,
                active,
                held_bytes,
            })
        })
        .collect()
}

/// The replication slot a standby holds on the primary.
///
/// Named for the slot it occupies, which is one thing across the whole
/// network — so the primary's slot list reads against the placement
/// page with nothing to map between. Postgres allows lowercase letters,
/// digits and underscores, and nothing else.
pub fn slot_name(slot: u32) -> String {
    format!("wabot_slot_{slot}")
}

/// What somebody pastes into an application.
pub fn connection_url(
    user: &str,
    password: &str,
    host: &str,
    port: u16,
    database_name: &str,
) -> String {
    format!("postgresql://{user}:{password}@{host}:{port}/{database_name}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::presets;

    /// The primary is asked about *slots*, not about connected
    /// standbys, and the reason is that absence is not an answer:
    /// `pg_stat_replication` omits a standby that stopped following,
    /// which reads exactly like a standby nobody ever made. A slot is a
    /// row either way and says `active`.
    #[test]
    fn what_the_primary_says_about_a_standby_that_stopped() {
        let following = parse_replication("wabot_slot_2|t|0\nwabot_slot_3|t|16777216\n");
        assert_eq!(
            following,
            vec![
                SlotState {
                    slot: 2,
                    active: true,
                    held_bytes: 0
                },
                SlotState {
                    slot: 3,
                    active: true,
                    held_bytes: 16_777_216
                },
            ]
        );

        // The failure this exists for: the container is up, the slot is
        // there, and nothing is connected to it.
        let stopped = parse_replication("wabot_slot_2|f|536870912");
        assert!(!stopped[0].active);
        assert_eq!(stopped[0].held_bytes, 536_870_912);

        // Tolerant, because this reads the output of a program in a
        // container: a notice, a truncated line or somebody's own slot
        // must not lose the rows that did parse.
        let noisy = parse_replication(
            "NOTICE: something\nwabot_slot_2|t|0\nsomebody_elses_slot|t|0\nwabot_slot_|t|0\n",
        );
        assert_eq!(noisy.len(), 1);
        assert_eq!(noisy[0].slot, 2);

        // And a missing byte count keeps the signal.
        assert!(!parse_replication("wabot_slot_4|f")[0].active);
    }

    /// Unpacking refuses a directory that already holds a database, and
    /// leaves the signal that makes the server replay.
    ///
    /// The signal is the step whose absence is invisible: without it the
    /// server opens the restored files as an ordinary database and
    /// replays nothing, which looks like a successful restore to the
    /// moment of the backup — and is silently not the moment that was
    /// asked for.
    #[test]
    fn unpacking_refuses_a_directory_that_is_not_empty() {
        let script = unpack_base().join(" ");

        assert!(script.starts_with("sh -c"), "{script}");
        // `set -e`, so a failed tar does not leave a half-cluster
        // looking unpacked.
        assert!(script.contains("set -e"), "{script}");
        assert!(script.contains("is not empty"), "{script}");
        // Both halves of what `pg_basebackup -Ft` produced.
        assert!(script.contains("base.tar.gz"), "{script}");
        assert!(script.contains("pg_wal.tar.gz"), "{script}");
        // The instruction to replay.
        assert!(script.contains(RECOVERY_SIGNAL), "{script}");
        // And the ownership, learned twice already today.
        assert!(script.contains("chown -R postgres:postgres"), "{script}");
        assert!(script.contains("chmod 0700"), "{script}");
    }

    /// A restore replays somebody else's log and must never write to
    /// it. Two halves: the archive is mounted read-only (the deploy
    /// path's job) and archiving is off in the arguments (this one's) —
    /// because a promoted database starts a new timeline, and archiving
    /// that into the original's directory would put a second history
    /// beside the first.
    #[test]
    fn a_restored_database_reads_the_archive_and_never_writes_it() {
        let arguments = recovery_arguments(256 * MB, Some("2026-08-16 14:32:00")).join(" ");

        assert!(
            arguments.contains("restore_command=gunzip -c"),
            "{arguments}"
        );
        assert!(
            arguments.contains("recovery_target_time=2026-08-16 14:32:00"),
            "{arguments}"
        );
        // Promote, not pause: the default waits for somebody to say what
        // happens next, which is a server that comes up refusing
        // connections with no obvious way out of it.
        assert!(
            arguments.contains("recovery_target_action=promote"),
            "{arguments}"
        );
        assert!(arguments.contains("archive_mode=off"), "{arguments}");
        // And it is still a database: the same ceiling arithmetic and
        // the same TLS as any other.
        assert!(arguments.contains("shared_buffers="), "{arguments}");
        assert!(arguments.contains("ssl=on"), "{arguments}");

        // No target is "as far as the log goes", which is what a node
        // being rebuilt wants.
        let latest = recovery_arguments(256 * MB, None).join(" ");
        assert!(!latest.contains("recovery_target_time"), "{latest}");
        assert!(latest.contains("restore_command="), "{latest}");
    }

    /// The archive command refuses to overwrite, and the reason is the
    /// one the documentation is unusually blunt about: a retry after a
    /// partial write would replace good bytes with truncated ones, and
    /// the copy it replaces is the one a restore needs.
    ///
    /// It also writes to a temporary name and moves it, so a segment
    /// that appears in the archive is a segment that is complete —
    /// there is no window in which the file exists and is half of one.
    #[test]
    fn archiving_never_overwrites_and_never_leaves_half_a_segment() {
        let command = archive_command();

        assert!(command.starts_with("test ! -f "), "{command}");
        assert!(command.contains(".tmp && mv "), "{command}");
        // Compressed, which is what makes a minute of an idle database
        // cost kilobytes instead of sixteen megabytes.
        assert!(command.contains("gzip"), "{command}");
        // And into the mount, never into the data directory: the whole
        // point is that it outlives the container.
        assert!(command.contains(ARCHIVE_MOUNT), "{command}");
    }

    /// Turning archiving off is written, not omitted. A deployment that
    /// left the setting out would leave whatever the last one set — so
    /// "off" would mean "unchanged", which is the one thing it must not
    /// mean.
    #[test]
    fn a_primary_says_whether_it_is_archiving_either_way() {
        let on = primary_arguments(256 * MB, true).join(" ");
        assert!(on.contains("archive_mode=on"), "{on}");
        assert!(on.contains("archive_timeout=60s"), "{on}");
        assert!(on.contains("wal_level=replica"), "{on}");

        let off = primary_arguments(256 * MB, false).join(" ");
        assert!(off.contains("archive_mode=off"), "{off}");
        assert!(!off.contains("archive_command"), "{off}");
    }

    /// A backup takes a *temporary* slot and a seed takes a permanent
    /// one, and the difference is the whole lifetime question: a seed's
    /// slot exists so the primary keeps log for a standby that will come
    /// back, where a backup's must hold nothing once the copy is done.
    /// `-X stream` takes one for the length of the copy and no longer.
    #[test]
    fn a_backup_holds_no_slot_after_it_is_taken() {
        let command = base_backup_into("10.42.2.200", PORT, "orders").join(" ");

        assert!(command.starts_with("pg_basebackup "), "{command}");
        assert!(
            !command.contains("--create-slot") && !command.contains("--slot"),
            "a backup must not leave a slot holding write-ahead log: {command}"
        );
        assert!(command.contains("--wal-method stream"), "{command}");
        // Tar, because the destination this is built for is object
        // storage: two objects rather than thousands of small files.
        assert!(command.contains("--format=tar"), "{command}");
        assert!(command.contains("--gzip"), "{command}");
        // And it does not wait for a natural checkpoint, because
        // somebody is standing in front of it.
        assert!(command.contains("--checkpoint=fast"), "{command}");
        assert!(!command.contains("PGPASSWORD"), "{command}");
    }

    /// A slot outlives the copy it was made for, because it lives on
    /// the *primary* — so reseeding a standby that has lived before
    /// meets `pg_basebackup -C` refusing: "replication slot
    /// "wabot_slot_2" already exists". Jorge hit it one step after the
    /// data directory was fixed, which is the same lesson twice: what a
    /// copy leaves behind is not all on the machine that held it.
    ///
    /// Dropped over the replication protocol, which is the one
    /// connection the primary already admits from a standby — a SQL
    /// login would have meant widening `pg_hba.conf` for every node
    /// holding a copy.
    #[test]
    fn seeding_drops_the_slot_it_is_about_to_create() {
        let command = base_backup("10.42.0.1", 30000, "wabot_replication", &slot_name(2));
        let script = command.last().expect("a script");

        assert_eq!(command.first().map(String::as_str), Some("sh"));
        assert!(
            script.contains("DROP_REPLICATION_SLOT wabot_slot_2"),
            "{script}"
        );
        assert!(
            script.contains("replication=database"),
            "over the replication connection, not a SQL one: {script}"
        );
        // Expected to fail on a standby seeded for the first time, and
        // the backup is what the container is for.
        assert!(script.contains("|| true"), "{script}");
        assert!(
            script.contains("pg_basebackup --host 10.42.0.1 --port 30000"),
            "{script}"
        );
        assert!(
            script.contains("--create-slot --slot wabot_slot_2"),
            "{script}"
        );
        // The password is never in the command: a command is in the
        // container's spec on disk and in `ctr containers info`.
        assert!(!script.contains("PGPASSWORD"), "{script}");
    }

    /// The mistake the whole table exists to prevent: `shared_buffers`
    /// defaults to 128 MB, which is twice the smallest rung. A ceiling
    /// without the arithmetic is a container killed before it starts.
    #[test]
    fn no_preset_asks_for_more_shared_memory_than_it_is_allowed() {
        for rung in presets::LADDER {
            let tuning = tuning(rung);
            let mb = rung / MB;
            assert!(
                tuning.shared_buffers_mb < mb,
                "{mb} MB: shared_buffers is {} MB",
                tuning.shared_buffers_mb
            );
            // And it leaves room for the backends, the WAL buffers and
            // the postmaster itself, none of which come out of
            // `shared_buffers`.
            assert!(
                tuning.shared_buffers_mb <= mb / 4,
                "{mb} MB: shared_buffers takes more than a quarter"
            );
        }
    }

    /// Inside a cgroup the page cache is charged to the cgroup, so the
    /// planner cannot be told there is more cache than the ceiling —
    /// and the ceiling is also holding `shared_buffers`.
    #[test]
    fn the_planner_is_never_told_about_cache_the_cgroup_would_not_allow() {
        for rung in presets::LADDER {
            let tuning = tuning(rung);
            assert!(
                (tuning.effective_cache_size_mb + tuning.shared_buffers_mb) <= rung / MB,
                "{} MB: the planner is promised more than the container has",
                rung / MB
            );
        }
    }

    #[test]
    fn the_smallest_rung_is_the_one_that_says_ten_connections() {
        let tuning = tuning(presets::LADDER[0]);
        assert_eq!(tuning.max_connections, 10);
        assert_eq!(tuning.shared_buffers_mb, 16);
        assert_eq!(tuning.work_mem_mb, 1);
        assert_eq!(tuning.parallel_workers, 0, "a worker is a process");
    }

    /// Parallel query is the classic container failure — each worker
    /// wants a shared memory segment — so it stays off until there is
    /// room for one.
    #[test]
    fn parallel_query_arrives_with_a_gigabyte() {
        assert_eq!(tuning(512 * MB).parallel_workers, 0);
        assert_eq!(tuning(1024 * MB).parallel_workers, 2);
    }

    /// Every rung has to produce arguments Postgres will accept, and
    /// the shape is `-c name=value` in pairs.
    #[test]
    fn the_arguments_are_pairs_the_entrypoint_passes_through() {
        for rung in presets::LADDER {
            let arguments = primary_arguments(rung, false);
            assert_eq!(arguments.len() % 2, 0, "an odd number of arguments");
            for pair in arguments.chunks(2) {
                assert_eq!(pair[0], "-c");
                assert!(pair[1].contains('='), "{:?} is not a setting", pair[1]);
            }
        }
    }

    /// Nothing over the network without TLS. The whole reason the
    /// server does its own encryption rather than something in front of
    /// it: a container on the project's bridge reaches this one
    /// directly, and no proxy can stand between them.
    #[test]
    fn no_line_would_accept_an_unencrypted_connection() {
        let file = hba(
            "10.42.2.0/24",
            "wabot_replication",
            &["10.42.0.5/32".to_string()],
            true,
        );

        for line in file.lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            // The socket is the exception, and it is not a network: it
            // lives inside the container, and the image's own entrypoint
            // cannot finish `initdb` without it.
            if line.starts_with("local ") {
                continue;
            }
            assert!(
                line.starts_with("hostssl"),
                "a line would take a plaintext connection: {line}"
            );
        }
        assert!(file.contains("hostssl"), "{file}");
        assert!(!file.contains("\nhost "), "{file}");
    }

    /// A standby has to ask for what the primary demands.
    ///
    /// These two are written in different functions and were changed
    /// months apart in spirit: `conninfo` said `sslmode=disable`
    /// because it predates the server doing TLS at all, and turning
    /// `hostssl` on for every line of `pg_hba.conf` broke replication
    /// the same afternoon. Silently — a standby that cannot connect is
    /// a container that stays up and says nothing, and the only trace
    /// was the primary's own log repeating `no pg_hba.conf entry …
    /// no encryption` every five seconds.
    ///
    /// So the two are asserted against each other rather than each
    /// against itself.
    #[test]
    fn what_a_standby_asks_for_is_what_the_primary_demands() {
        let demanded = hba(
            "10.42.2.0/24",
            "wabot_replication",
            &["10.42.0.4/32".to_string()],
            false,
        );
        let requires_tls = demanded
            .lines()
            .any(|line| line.starts_with("hostssl replication"));
        assert!(requires_tls, "{demanded}");

        let asked = conninfo("10.42.0.1", 30002, "wabot_replication", "secret");
        assert!(
            !asked.contains("sslmode=disable"),
            "the primary refuses this connection: {asked}"
        );
        assert!(
            asked.contains("sslmode=require") || asked.contains("sslmode=verify"),
            "{asked}"
        );
    }

    /// The server is told where its own certificate is, on every start,
    /// so a renewal arrives by the same path that placed the first one.
    #[test]
    fn the_server_is_pointed_at_its_certificate() {
        let arguments = primary_arguments(128 * 1024 * 1024, false).join(" ");
        assert!(arguments.contains("-c ssl=on"), "{arguments}");
        assert!(
            arguments.contains(&format!("ssl_cert_file={}", certificate_path())),
            "{arguments}"
        );
        assert!(
            arguments.contains(&format!("ssl_key_file={}", key_path())),
            "{arguments}"
        );
        // Beside the data directory, not inside it: `initdb` refuses a
        // data directory that is not empty, and these are written before
        // the first start.
        assert!(TLS_DIR.starts_with(DATA_MOUNT));
        assert!(!TLS_DIR.starts_with(PGDATA));
    }

    /// The one the image's own configuration would get wrong, and the
    /// reason the node writes the file at all.
    #[test]
    fn the_hba_lets_a_standby_in_where_all_would_not() {
        let file = hba(
            "10.42.3.0/24",
            "wabot_replication",
            &["10.42.0.5/32".to_string()],
            false,
        );

        assert!(
            file.contains("hostssl replication   wabot_replication"),
            "{file}"
        );
        assert!(file.contains("10.42.0.5/32"), "{file}");
        // Without this the entrypoint's temporary server cannot
        // connect over its own socket, and `initdb` never finishes.
        assert!(file.contains("local   all           all"), "{file}");
        assert!(
            !file.contains("0.0.0.0/0"),
            "an unpublished database was opened to the world"
        );
    }

    /// A standby on *this* node is seeded from the host's network
    /// namespace, so it arrives from the bridge gateway — and the
    /// subnet line for `all` does not cover a replication connection.
    ///
    /// The node said this six times before anybody read it:
    /// `no pg_hba.conf entry for replication connection from host
    /// "10.42.2.1"`. The design had claimed a local copy "arrives over
    /// the project's own bridge, which the subnet line already covers",
    /// which was wrong about both halves.
    #[test]
    fn a_standby_on_this_node_is_let_in_from_the_bridge_gateway() {
        let file = hba(
            "10.42.2.0/24",
            "wabot_replication",
            // What the deploy path passes when a copy runs here.
            &["10.42.2.0/24".to_string()],
            false,
        );

        let replication: Vec<&str> = file
            .lines()
            .filter(|line| line.starts_with("hostssl replication"))
            .collect();
        assert_eq!(replication.len(), 1, "{file}");
        assert!(replication[0].contains("10.42.2.0/24"), "{file}");

        // The gateway is inside it, which is the only reason one line
        // does for both the seed and the standby container.
        let gateway: std::net::Ipv4Addr = "10.42.2.1".parse().unwrap();
        assert_eq!(gateway.octets()[..3], [10, 42, 2]);
    }

    #[test]
    fn a_published_database_admits_a_client_from_anywhere() {
        let file = hba("10.42.3.0/24", "repl", &[], true);
        assert!(file.contains("0.0.0.0/0"), "{file}");
    }

    /// A standby's line is one per node, not one per copy: two copies
    /// on one node arrive from the same address.
    #[test]
    fn every_standby_node_gets_its_own_line() {
        let file = hba(
            "10.42.3.0/24",
            "repl",
            &["10.42.0.5/32".to_string(), "10.42.0.9/32".to_string()],
            false,
        );
        assert_eq!(file.matches("hostssl replication").count(), 2, "{file}");
    }

    /// A generated password goes into a single-quoted SQL literal and
    /// into a conninfo, and it is alphanumeric — so neither needs
    /// escaping. This is the test that keeps that true.
    #[test]
    fn a_generated_password_needs_no_escaping_anywhere() {
        for _ in 0..64 {
            let password = wabot::prelude::password::generate(24);
            assert!(
                password.chars().all(|c| c.is_ascii_alphanumeric()),
                "{password} would have to be escaped"
            );
        }
    }

    #[test]
    fn the_image_is_fully_qualified_because_containerd_does_not_guess() {
        assert_eq!(
            image_for("17"),
            "docker.io/library/postgres:17-alpine",
            "a short reference names a registry called `postgres`"
        );
        assert!(VERSIONS.contains(&DEFAULT_VERSION));
    }

    /// `initdb` refuses to own a bind-mounted directory, so `PGDATA` is
    /// one level below the mount point. Getting this wrong is a
    /// database that will not initialise and says so in a way that
    /// reads like a permissions bug.
    #[test]
    fn the_data_directory_is_below_the_mount_point() {
        assert!(PGDATA.starts_with(DATA_MOUNT));
        assert_ne!(PGDATA, DATA_MOUNT);
    }

    #[test]
    fn a_slot_name_is_one_postgres_accepts() {
        let name = slot_name(12);
        assert_eq!(name, "wabot_slot_12");
        assert!(name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'));
    }
}
