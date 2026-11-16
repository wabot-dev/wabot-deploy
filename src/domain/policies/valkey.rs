//! What a Valkey container is told, derived from its mode and its ceiling.
//!
//! Every number here was measured on a VM node under the ceiling it is
//! written against — `docs/databases.md`, "Valkey", has the runs.

use sha2::{Digest, Sha256};

use crate::domain::entities::database::Mode;

const MB: u64 = 1024 * 1024;

pub fn image_for(version: &str) -> String {
    format!("docker.io/valkey/valkey:{version}-alpine")
}

pub const VERSIONS: [&str; 1] = ["9.1"];

pub const DEFAULT_VERSION: &str = "9.1";

/// `valkey:9.1-alpine` says `VALKEY_VERSION=9.1.3`: the release, where a row
/// keeps the series.
pub const VERSION_VARIABLE: &str = "VALKEY_VERSION";

pub const REDIS_VARIABLE: &str = "REDIS_VERSION";

pub const PORT: u16 = 6379;

pub const DATA_MOUNT: &str = "/data";

/// Inside the data directory, because the image's entrypoint gives
/// everything under it to `valkey` before it drops root.
pub const TLS_DIR: &str = "/data/tls";

pub const SERVER_USER: &str = "valkey";

pub const CONFIG_FILE: &str = "valkey.conf";

pub const ACL_FILE: &str = "users.acl";

pub const BACKUP_MOUNT: &str = "/backup";

pub const DUMP_FILE: &str = "dump.rdb";

pub const AOF_DIR: &str = "appendonlydir";

pub const AOF_MANIFEST: &str = "appendonly.aof.manifest";

pub const AOF_BASE: &str = "appendonly.aof.1.base.rdb";

/// A dump `import` left for the next deployment to start from.
pub const PENDING_IMPORT: &str = ".wabot-import.rdb";

/// What Valkey 9.1 writes and the newest it reads. Redis 7.4 moved to 12,
/// and an RDB from Redis 8.10 says 15: measured, Valkey stops with "Can't
/// handle RDB format version 15".
pub const NEWEST_FORMAT: u32 = 11;

pub const FLOOR: u64 = 64 * MB;

/// An idle server's resident size with room to spare, and the distance
/// `memory.high` keeps from the ceiling. Without that distance a burst of
/// writes filled a 64 MB store with 36 MB of dirty AOF pages that nothing
/// had started writing back, and the kernel killed it; with it every rung
/// survived the same burst.
pub const HEADROOM: u64 = 16 * MB;

/// What a container's 1,024 open files leave once Valkey keeps 32 for
/// itself. Its default of 10,000 was cut to this at start, with a warning
/// that it could not raise the limit, on every VM node run.
pub const MAX_CLIENTS: u32 = 992;

/// A renewed certificate is served within this many seconds without the
/// node asking for it.
pub const TLS_RELOAD_SECONDS: u32 = 60;

/// Two fifths for a store, because rewriting the AOF and the backup both
/// fork, and a fork under writes copies nearly every page it is holding.
/// Three quarters for a cache, which never forks.
pub fn maxmemory(mode: Mode, memory_limit: u64) -> u64 {
    let room = memory_limit.max(FLOOR).saturating_sub(HEADROOM);
    match mode {
        Mode::Store => room * 2 / 5,
        Mode::Cache => room * 3 / 4,
    }
}

pub fn memory_high(memory_limit: u64) -> u64 {
    memory_limit.max(FLOOR).saturating_sub(HEADROOM)
}

pub fn config(mode: Mode, memory_limit: u64, config_mount: &str) -> String {
    let (policy, appendonly) = match mode {
        Mode::Store => ("noeviction", "yes"),
        Mode::Cache => ("allkeys-lru", "no"),
    };
    format!(
        "port 0\n\
         tls-port {PORT}\n\
         tls-cert-file {TLS_DIR}/server.crt\n\
         tls-key-file {TLS_DIR}/server.key\n\
         tls-auth-clients no\n\
         tls-auto-reload-interval {TLS_RELOAD_SECONDS}\n\
         aclfile {config_mount}/{ACL_FILE}\n\
         dir {DATA_MOUNT}\n\
         maxmemory {}\n\
         maxmemory-policy {policy}\n\
         maxclients {MAX_CLIENTS}\n\
         save \"\"\n\
         appendonly {appendonly}\n\
         appendfsync everysec\n\
         appendfilename \"appendonly.aof\"\n\
         appenddirname \"{AOF_DIR}\"\n",
        maxmemory(mode, memory_limit),
    )
}

/// The users, with their passwords as SHA-256: the file is mounted where
/// the server's own user can read it.
///
/// `default` is written out as off, because a file that leaves it out gets
/// it back with no password and every command.
pub fn acl(app_user: &str, app_password: &str, node_user: &str, node_password: &str) -> String {
    format!(
        "user default off resetchannels -@all\n\
         user {app_user} on #{} ~* &* +@all -@admin +config|get\n\
         user {node_user} on #{} resetchannels -@all +auth +hello +ping +info +sync +psync +replconf\n",
        sha256(app_password),
        sha256(node_password),
    )
}

fn sha256(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn cli(host: &str, port: u16, user: &str) -> Vec<String> {
    [
        "valkey-cli",
        "-h",
        host,
        "-p",
        &port.to_string(),
        "--tls",
        "--insecure",
        "--user",
        user,
    ]
    .iter()
    .map(|argument| argument.to_string())
    .collect()
}

/// The password goes in `REDISCLI_AUTH`, never on the command line.
pub const PASSWORD_VARIABLE: &str = "REDISCLI_AUTH";

pub fn dump_into(host: &str, port: u16, user: &str) -> Vec<String> {
    let mut command = cli(host, port, user);
    command.extend(["--rdb".to_string(), format!("{BACKUP_MOUNT}/{DUMP_FILE}")]);
    command
}

pub fn keyspace(host: &str, port: u16, user: &str) -> Vec<String> {
    let mut command = cli(host, port, user);
    command.extend(["INFO".to_string(), "keyspace".to_string()]);
    command
}

/// Every key in every numbered database, or `None` when what was said is
/// not an answer to `INFO keyspace`.
pub fn keys_in(said: &str) -> Option<u64> {
    if !said.lines().any(|line| line.trim() == "# Keyspace") {
        return None;
    }
    Some(
        said.lines()
            .filter(|line| line.starts_with("db"))
            .filter_map(|line| {
                line.split_once(':')?
                    .1
                    .split(',')
                    .find_map(|field| field.strip_prefix("keys="))?
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .sum(),
    )
}

/// The format version an RDB file declares in its first nine bytes.
pub fn rdb_format(header: &[u8]) -> Option<u32> {
    let digits = header.strip_prefix(b"REDIS")?.get(..4)?;
    std::str::from_utf8(digits).ok()?.parse().ok()
}

/// What makes an AOF out of one RDB: Valkey loads the base, then appends
/// beside it. Measured, a server started with `appendonly yes` and only a
/// `dump.rdb` ignores the dump and starts empty.
pub fn manifest() -> String {
    format!("file {AOF_BASE} seq 1 type b\n")
}

pub fn connection_url(user: &str, password: &str, host: &str, port: u16) -> String {
    format!("rediss://{user}:{password}@{host}:{port}/0")
}

pub fn identifier(of_slug: &str, node_user: &str) -> String {
    match ["default", node_user].contains(&of_slug) {
        true => format!("db_{of_slug}"),
        false => of_slug.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policies::presets;

    fn rungs() -> impl Iterator<Item = u64> {
        presets::LADDER.into_iter().filter(|rung| *rung >= FLOOR)
    }

    #[test]
    fn a_store_leaves_room_for_a_fork_to_copy_everything_it_holds() {
        for rung in rungs() {
            let held = maxmemory(Mode::Store, rung);
            assert!(HEADROOM + 2 * held + held / 5 <= rung, "{}", rung / MB);
        }
    }

    #[test]
    fn a_cache_holds_more_than_a_store_of_the_same_size_and_still_fits() {
        for rung in rungs() {
            let cache = maxmemory(Mode::Cache, rung);
            assert!(cache > maxmemory(Mode::Store, rung), "{}", rung / MB);
            assert!(HEADROOM + cache + cache / 5 <= rung, "{}", rung / MB);
        }
    }

    #[test]
    fn the_kernel_starts_pushing_back_before_the_ceiling() {
        for rung in rungs() {
            assert!(memory_high(rung) < rung);
            assert!(memory_high(rung) > maxmemory(Mode::Cache, rung));
        }
    }

    #[test]
    fn under_the_floor_a_server_is_sized_as_the_floor() {
        assert_eq!(maxmemory(Mode::Store, 0), maxmemory(Mode::Store, FLOOR));
        assert_eq!(memory_high(32 * MB), memory_high(FLOOR));
    }

    #[test]
    fn nothing_is_reachable_in_the_clear() {
        for mode in [Mode::Store, Mode::Cache] {
            let config = config(mode, FLOOR, "/etc/wabot");
            assert!(config.lines().any(|line| line == "port 0"), "{config}");
            assert!(config.contains(&format!("tls-port {PORT}")), "{config}");
        }
    }

    /// `tls-auth-clients` is `yes` unless it is said otherwise, and that
    /// asks every client for a certificate of its own.
    #[test]
    fn no_client_is_asked_for_a_certificate() {
        let config = config(Mode::Store, FLOOR, "/etc/wabot");
        assert!(config.contains("tls-auth-clients no\n"), "{config}");
    }

    #[test]
    fn a_renewed_certificate_is_picked_up_without_being_asked() {
        let config = config(Mode::Store, FLOOR, "/etc/wabot");
        assert!(config.contains("tls-auto-reload-interval "), "{config}");
    }

    #[test]
    fn a_cache_writes_nothing_and_a_store_keeps_every_acknowledged_second() {
        let cache = config(Mode::Cache, FLOOR, "/etc/wabot");
        assert!(cache.contains("appendonly no\n") && cache.contains("save \"\"\n"));
        assert!(cache.contains("maxmemory-policy allkeys-lru\n"));

        let store = config(Mode::Store, FLOOR, "/etc/wabot");
        assert!(store.contains("appendonly yes\n") && store.contains("appendfsync everysec\n"));
        assert!(store.contains("maxmemory-policy noeviction\n"));
    }

    #[test]
    fn the_server_reads_its_users_from_the_file_the_node_writes() {
        let config = config(Mode::Store, FLOOR, "/etc/wabot");
        assert!(config.contains(&format!("aclfile /etc/wabot/{ACL_FILE}\n")));
    }

    #[test]
    fn no_password_is_written_down_and_default_cannot_log_in() {
        let acl = acl("orders", "app-secret", "wabot_replication", "node-secret");
        assert!(
            !acl.contains("app-secret") && !acl.contains("node-secret"),
            "{acl}"
        );
        assert!(
            acl.lines().any(|line| line.starts_with("user default off")),
            "{acl}"
        );
        assert!(acl.contains(&format!("#{}", sha256("app-secret"))), "{acl}");
    }

    #[test]
    fn the_application_owns_its_data_and_not_the_server() {
        let acl = acl("orders", "a", "wabot_replication", "n");
        let app = acl
            .lines()
            .find(|line| line.starts_with("user orders "))
            .expect("the application's line");
        let admin = app.find("-@admin").expect("administration taken away");
        assert!(app.find("+@all").is_some_and(|all| all < admin), "{app}");
        assert!(!app[admin..].contains("+@admin"), "{app}");
    }

    #[test]
    fn the_node_can_take_a_backup_and_nothing_else() {
        let acl = acl("orders", "a", "wabot_replication", "n");
        let node = acl
            .lines()
            .find(|line| line.starts_with("user wabot_replication "))
            .expect("the node's line");
        assert!(node.contains("-@all") && node.contains("+sync") && node.contains("+replconf"));
        assert!(!node.contains("~"), "the node reads no keys: {node}");
    }

    #[test]
    fn a_backup_or_a_count_never_puts_a_password_on_the_command_line() {
        for command in [
            dump_into("10.42.2.200", PORT, "wabot_replication"),
            keyspace("10.42.2.200", PORT, "wabot_replication"),
        ] {
            let joined = command.join(" ");
            assert!(
                !joined.contains(" -a ") && !joined.contains("--pass"),
                "{joined}"
            );
            assert!(joined.contains("--tls"), "{joined}");
        }
        assert!(dump_into("h", PORT, "u")
            .join(" ")
            .ends_with("--rdb /backup/dump.rdb"));
    }

    #[test]
    fn keys_are_counted_across_every_numbered_database() {
        let said =
            "# Keyspace\r\ndb0:keys=3,expires=0,avg_ttl=0\r\ndb4:keys=2,expires=1,avg_ttl=9\r\n";
        assert_eq!(keys_in(said), Some(5));
        assert_eq!(keys_in("# Keyspace\r\n"), Some(0));
        assert_eq!(keys_in("NOPERM User has no permissions"), None);
    }

    #[test]
    fn a_dump_from_redis_8_is_told_apart_from_one_valkey_reads() {
        assert_eq!(rdb_format(b"REDIS0011\xfa"), Some(11));
        assert_eq!(rdb_format(b"REDIS0015"), Some(15));
        assert!(rdb_format(b"REDIS0015").is_some_and(|format| format > NEWEST_FORMAT));
        assert_eq!(rdb_format(b"PK\x03\x04"), None);
        assert_eq!(rdb_format(b"REDIS"), None);
    }

    #[test]
    fn the_string_says_which_user_and_asks_for_tls() {
        let url = connection_url("orders", "pw", "orders.shop", PORT);
        assert_eq!(url, "rediss://orders:pw@orders.shop:6379/0");
    }

    #[test]
    fn an_identifier_is_never_a_user_valkey_or_the_node_already_has() {
        assert_eq!(identifier("orders", "wabot_replication"), "orders");
        assert_eq!(identifier("default", "wabot_replication"), "db_default");
        assert_eq!(
            identifier("wabot_replication", "wabot_replication"),
            "db_wabot_replication"
        );
    }
}
