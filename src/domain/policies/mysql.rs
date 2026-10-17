//! What a MySQL container is told, derived from its preset.
//!
//! Every number here was measured on a VM node under the ceiling it is
//! written against — `docs/databases.md`, "MySQL", has the runs.

const MB: u64 = 1024 * 1024;

pub fn image_for(version: &str) -> String {
    format!("docker.io/library/mysql:{version}")
}

pub const VERSIONS: [&str; 1] = ["8.4"];

pub const DEFAULT_VERSION: &str = "8.4";

pub const PORT: u16 = 3306;

pub const DATA_MOUNT: &str = "/var/lib/mysql";

/// One level below the mount, because `--initialize` refuses a directory
/// that already holds the certificate the node writes before the first
/// start.
pub const DATADIR: &str = "/var/lib/mysql/data";

pub const TLS_DIR: &str = "/var/lib/mysql/tls";

pub const SERVER_USER: &str = "mysql";

pub const BASE_MOUNT: &str = "/base";

pub const BACKUP_MOUNT: &str = "/backup";

pub const DUMP_FILE: &str = "dump.sql.gz";

/// Inside the data directory, because it is the one place the server's
/// own user may write while the entrypoint runs the restore.
pub const RESTORED_MARK: &str = ".wabot-restored";

/// Under it the kernel kills the server while `--initialize` is still
/// writing its dictionary, whatever the server is told: 128 MB was
/// killed with a 16 MB pool.
pub const FLOOR: u64 = 256 * MB;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tuning {
    pub buffer_pool_mb: u64,
    pub log_buffer_mb: u64,
    pub temptable_mb: u64,
    pub tmp_table_mb: u64,
    pub max_connections: u32,
}

/// The rungs to 1 GB peak at 77–83 % of their ceiling with every
/// connection sorting or grouping at once; the two above them are the
/// same arithmetic on a machine too small to run them.
pub fn tuning(memory_limit: u64) -> Tuning {
    let (buffer_pool_mb, log_buffer_mb, temptable_mb, tmp_table_mb, max_connections) =
        match memory_limit.max(FLOOR) / MB {
            0..=511 => (48, 8, 16, 8, 20),
            512..=1023 => (192, 16, 32, 16, 40),
            1024..=2047 => (512, 16, 64, 16, 80),
            2048..=4095 => (1024, 32, 128, 16, 150),
            _ => (2304, 64, 256, 16, 200),
        };
    Tuning {
        buffer_pool_mb,
        log_buffer_mb,
        temptable_mb,
        tmp_table_mb,
        max_connections,
    }
}

pub fn arguments(memory_limit: u64) -> Vec<String> {
    let tuning = tuning(memory_limit);
    vec![
        format!("--datadir={DATADIR}"),
        format!("--innodb-buffer-pool-size={}M", tuning.buffer_pool_mb),
        // The pool is rounded up to a multiple of this, and the stock
        // 128 MB would turn 192 into 256.
        "--innodb-buffer-pool-chunk-size=16M".to_string(),
        // The stock 64 MB is a quarter of the smallest ceiling.
        format!("--innodb-log-buffer-size={}M", tuning.log_buffer_mb),
        // The stock gigabyte is shared by every connection and is four
        // times the smallest ceiling; past this it spills to disk.
        format!("--temptable-max-ram={}M", tuning.temptable_mb),
        format!("--tmp-table-size={}M", tuning.tmp_table_mb),
        format!("--max-heap-table-size={}M", tuning.tmp_table_mb),
        format!("--max-connections={}", tuning.max_connections),
        // It sizes itself from the connection count and the table caches,
        // which is memory no rung would be choosing.
        "--performance-schema=OFF".to_string(),
        "--mysqlx=OFF".to_string(),
        // Thirty days of disk by default, and nothing reads it until
        // there are replicas or point-in-time recovery.
        "--disable-log-bin".to_string(),
        // The socket the entrypoint initialises over is exempt, so this
        // refuses exactly what `hostssl` refuses for Postgres.
        "--require-secure-transport=ON".to_string(),
        format!("--ssl-cert={TLS_DIR}/server.crt"),
        format!("--ssl-key={TLS_DIR}/server.key"),
        // It checks its own certificate against these and refuses a reload
        // it cannot check, where a start only warns — which is how a node
        // found it. The node's authority signs its own; the image's roots
        // sign a public one.
        format!("--ssl-ca={}/ca.crt", super::postgres::CONFIG_MOUNT),
        format!("--ssl-capath={TRUSTED_ROOTS}"),
    ]
}

const TRUSTED_ROOTS: &str = "/etc/pki/ca-trust/extracted/pem/directory-hash";

pub fn environment(
    admin_user: &str,
    admin_password: &str,
    database_name: &str,
) -> Vec<(String, String)> {
    [
        ("MYSQL_ROOT_PASSWORD", admin_password),
        // Root is for whoever can reach the socket inside the container;
        // over the network the admin user is the door.
        ("MYSQL_ROOT_HOST", "localhost"),
        ("MYSQL_DATABASE", database_name),
        ("MYSQL_USER", admin_user),
        ("MYSQL_PASSWORD", admin_password),
        // glibc's per-thread arenas: without this the 256 MB rung was
        // killed by forty busy connections, and with it survived them.
        ("MALLOC_ARENA_MAX", "2"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), value.to_string()))
    .collect()
}

/// The node's own user, made once at initialisation.
///
/// Replication and cloning are granted before anything uses them, so a
/// read-only copy can be added later without SQL against a running
/// server — the reason the Postgres one has its role from birth.
pub fn init_script(node_user: &str, node_password: &str) -> String {
    format!(
        "CREATE USER '{node_user}'@'%' IDENTIFIED BY '{node_password}' REQUIRE SSL;\n\
         GRANT SELECT, SHOW VIEW, TRIGGER, EVENT, LOCK TABLES, SHOW_ROUTINE, \
         CONNECTION_ADMIN, REPLICATION SLAVE, BACKUP_ADMIN ON *.* TO '{node_user}'@'%';\n"
    )
}

/// Sourced by the image's entrypoint after it has made the users, so the
/// dump loads into the database it names and a failure stops the
/// entrypoint before the mark is written.
pub fn restore_script() -> String {
    format!(
        "mysql_note \"wabot-deploy: loading the backup\"\n\
         gunzip -c {BASE_MOUNT}/{DUMP_FILE} | docker_process_sql\n\
         touch \"$DATADIR/{RESTORED_MARK}\"\n"
    )
}

pub fn dump_into(host: &str, port: u16, user: &str, database_name: &str) -> Vec<String> {
    let dump = [
        "mysqldump",
        &format!("--host={host}"),
        &format!("--port={port}"),
        &format!("--user={user}"),
        "--ssl-mode=REQUIRED",
        "--single-transaction",
        "--routines",
        "--triggers",
        "--events",
        // A restore loads into a server with its own history, which a
        // `SET @@GLOBAL.GTID_PURGED` from this one would contradict.
        "--set-gtid-purged=OFF",
        "--no-tablespaces",
        database_name,
    ]
    .join(" ");
    vec![
        "sh".to_string(),
        "-c".to_string(),
        format!("set -o pipefail; {dump} | gzip > {BACKUP_MOUNT}/{DUMP_FILE}"),
    ]
}

/// A SIGHUP does not reread the certificate: measured, the server kept
/// serving the old one.
pub fn reload_tls(host: &str, port: u16, user: &str) -> Vec<String> {
    [
        "mysql",
        &format!("--host={host}"),
        &format!("--port={port}"),
        &format!("--user={user}"),
        "--ssl-mode=REQUIRED",
        "--execute=ALTER INSTANCE RELOAD TLS",
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect()
}

pub fn connection_url(
    user: &str,
    password: &str,
    host: &str,
    port: u16,
    database_name: &str,
) -> String {
    format!("mysql://{user}:{password}@{host}:{port}/{database_name}")
}

/// MySQL stops a user name at 32 characters, and the image refuses
/// `root`; the schemas it keeps for itself would hand their grant tables
/// to whoever got `ALL` on one.
pub fn identifier(of_slug: &str, node_user: &str) -> String {
    let mut identifier = of_slug.to_string();
    identifier.truncate(32);
    let reserved = [
        "root",
        "mysql",
        "sys",
        "information_schema",
        "performance_schema",
        node_user,
    ];
    if reserved.contains(&identifier.as_str()) {
        identifier.insert_str(0, "db_");
    }
    identifier
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policies::presets;

    #[test]
    fn what_every_rung_holds_for_good_fits_under_its_ceiling() {
        for rung in presets::LADDER.into_iter().filter(|rung| *rung >= FLOOR) {
            let tuning = tuning(rung);
            let held = (tuning.buffer_pool_mb + tuning.log_buffer_mb + tuning.temptable_mb) * MB;
            assert!(held < rung / 2 + rung / 4, "{} holds {held}", rung / MB);
        }
    }

    #[test]
    fn a_pool_is_never_rounded_past_what_was_chosen() {
        for rung in presets::LADDER {
            assert_eq!(tuning(rung).buffer_pool_mb % 16, 0, "{}", rung / MB);
        }
        assert!(arguments(FLOOR).contains(&"--innodb-buffer-pool-chunk-size=16M".to_string()));
    }

    #[test]
    fn under_the_floor_a_server_is_tuned_as_the_floor() {
        assert_eq!(tuning(64 * MB), tuning(FLOOR));
        assert_eq!(tuning(0), tuning(FLOOR));
    }

    #[test]
    fn a_server_refuses_what_is_not_encrypted() {
        let arguments = arguments(FLOOR);
        assert!(arguments.contains(&"--require-secure-transport=ON".to_string()));
        assert!(arguments.contains(&format!("--ssl-key={TLS_DIR}/server.key")));
        assert!(arguments.contains(&format!("--datadir={DATADIR}")));
    }

    /// Found on the VM node: with no authority named, a renewed
    /// certificate signed by the node's own was refused at
    /// `ALTER INSTANCE RELOAD TLS` as "Invalid certificates", and the
    /// server went on serving the old one.
    #[test]
    fn a_server_can_check_the_certificate_it_is_handed() {
        let arguments = arguments(FLOOR);
        assert!(arguments.contains(&"--ssl-ca=/etc/wabot/ca.crt".to_string()));
        assert!(arguments
            .iter()
            .any(|argument| argument.starts_with("--ssl-capath=")));
    }

    #[test]
    fn root_is_not_reachable_over_the_network() {
        let env: std::collections::BTreeMap<_, _> = environment("orders", "secret", "orders")
            .into_iter()
            .collect();
        assert_eq!(env["MYSQL_ROOT_HOST"], "localhost");
        assert_eq!(env["MYSQL_USER"], "orders");
        assert_eq!(env["MYSQL_DATABASE"], "orders");
    }

    #[test]
    fn a_dump_that_fails_part_way_is_a_backup_that_failed() {
        let command = dump_into("10.42.2.200", PORT, "wabot_replication", "orders").join(" ");
        assert!(command.contains("set -o pipefail"), "{command}");
        assert!(command.contains("--single-transaction"), "{command}");
        assert!(!command.contains("--password"), "{command}");
    }

    #[test]
    fn a_restore_is_marked_done_only_after_it_loaded() {
        let script = restore_script();
        let load = script.find("docker_process_sql").expect("a load");
        let mark = script.find(RESTORED_MARK).expect("a mark");
        assert!(load < mark, "{script}");
    }

    #[test]
    fn an_identifier_is_never_a_name_mysql_already_uses() {
        assert_eq!(identifier("orders", "wabot_replication"), "orders");
        assert_eq!(identifier("root", "wabot_replication"), "db_root");
        assert_eq!(identifier("mysql", "wabot_replication"), "db_mysql");
        assert_eq!(
            identifier("wabot_replication", "wabot_replication"),
            "db_wabot_replication"
        );
        assert_eq!(identifier(&"a".repeat(60), "wabot_replication").len(), 32);
    }
}
