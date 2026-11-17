//! What a Garage container is told, derived from its ceiling.
//!
//! The one place that knows the engine is Garage. Everything outside it
//! talks about stores, buckets and keys, so a permissive engine could take
//! its place — `docs/buckets.md`, "The licence".
//!
//! The memory numbers are provisional until the VM measures them under each
//! rung, as Valkey's were: `docs/buckets.md`, "Sizes".

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::bucket_key::Access;

const MB: u64 = 1024 * 1024;

pub const OFF: &str = "this node runs no stores of buckets until they are turned on in its \
     settings, under Capabilities";

pub const VERSIONS: [&str; 1] = ["2"];

pub const DEFAULT_VERSION: &str = "2";

pub const RELEASE: &str = "v2.5.0";

pub fn image_for(version: &str) -> Option<String> {
    VERSIONS
        .contains(&version)
        .then(|| format!("docker.io/dxflrs/garage:{RELEASE}"))
}

pub const S3_PORT: u16 = 3900;

pub const RPC_PORT: u16 = 3901;

pub const WEB_PORT: u16 = 3902;

/// Reached by the node at the container's address, and never routed.
pub const ADMIN_PORT: u16 = 3903;

pub const DATA_MOUNT: &str = "/var/lib/garage";

pub const DATA_VOLUME: &str = "data";

pub const METADATA_DIR: &str = "meta";

pub const SNAPSHOTS_DIR: &str = "snapshots";

/// What Garage names its SQLite metadata, in the live directory and in each
/// snapshot alike.
pub const METADATA_FILE: &str = "db.sqlite";

pub const CONFIG_FILE: &str = "garage.toml";

pub const RPC_SECRET_FILE: &str = "rpc_secret";

pub const ADMIN_TOKEN_FILE: &str = "admin_token";

/// Not one of Amazon's, because in `us-east-1` boto3 presigns with signature
/// version 2, which Amazon still takes there and Garage does not: the URL
/// works on Amazon and fails here. In a region boto3 does not know it signs
/// with version 4. Garage answers a request signed for another region with
/// the one it expected, which aws and boto3 sign again for by themselves and
/// any other client is refused by at its first request — at the desk, rather
/// than in production when a link is first opened. Measured 2026-11-17.
pub const REGION: &str = "wabot";

pub const FLOOR: u64 = 128 * MB;

/// Garage's own default, more than four of the rungs.
pub const DEFAULT_BLOCK_BUFFER: u64 = 256 * MB;

/// An eighth of the ceiling: what waits in RAM to reach the disk, beside a
/// process that also holds its metadata, its connections and each request's
/// own buffers, none of which this setting counts.
pub fn block_buffer(memory_limit: u64) -> u64 {
    let eighth = memory_limit.max(FLOOR) / 8;
    let whole_mebibytes = eighth / MB * MB;
    whole_mebibytes.clamp(8 * MB, DEFAULT_BLOCK_BUFFER)
}

/// Where the kernel starts reclaiming, below the ceiling. Blocks reach the
/// disk through the page cache, charged to the container, and dirty pages
/// nothing had started writing back are what killed a 64 MB Valkey store at
/// its ceiling.
pub fn memory_high(memory_limit: u64) -> u64 {
    let limit = memory_limit.max(FLOOR);
    limit - (limit / 8).max(16 * MB)
}

const CORS_METHODS: [&str; 5] = ["GET", "HEAD", "PUT", "POST", "DELETE"];

const INDEX_DOCUMENT: &str = "index.html";

const RULE_ID: &str = "wabot";

/// Everything `UpdateBucket` is told, from the row alone, so a second pass
/// says the same thing and an edit made in Garage by anybody else is put
/// back.
///
/// `ETag` is exposed because a browser uploading in parts reads it from each
/// part to complete the upload, and without it that fails in the browser and
/// works in `curl`.
pub fn bucket_settings(bucket: &Bucket) -> serde_json::Value {
    let website = match bucket.public {
        true => serde_json::json!({ "enabled": true, "indexDocument": INDEX_DOCUMENT }),
        false => serde_json::json!({ "enabled": false }),
    };
    let cors = match bucket.cors_origins.is_empty() {
        true => serde_json::json!([]),
        false => serde_json::json!([{
            "ID": RULE_ID,
            "AllowedOrigin": bucket.cors_origins.iter().map(|origin| origin.as_str()).collect::<Vec<_>>(),
            "AllowedMethod": CORS_METHODS,
            "AllowedHeader": ["*"],
            "ExposeHeader": ["ETag"],
            "MaxAgeSeconds": 3600,
        }]),
    };
    let mut rule = serde_json::json!({ "ID": RULE_ID, "Status": "Enabled" });
    if let Some(days) = bucket.expire_days {
        rule["Expiration"] = serde_json::json!({ "Days": days });
    }
    if let Some(days) = bucket.abort_uploads_days {
        rule["AbortIncompleteMultipartUpload"] = serde_json::json!({ "DaysAfterInitiation": days });
    }
    let lifecycle = match bucket.expire_days.is_some() || bucket.abort_uploads_days.is_some() {
        true => serde_json::json!([rule]),
        false => serde_json::json!([]),
    };
    serde_json::json!({
        "quotas": { "maxSize": bucket.max_size, "maxObjects": null },
        "websiteAccess": website,
        "corsRules": cors,
        "lifecycleRules": lifecycle,
    })
}

/// `--single-node` lays out a one-node cluster by itself: no layout to
/// assign, apply and remember.
pub fn command(config_mount: &str) -> Vec<String> {
    vec![
        "/garage".into(),
        "-c".into(),
        format!("{config_mount}/{CONFIG_FILE}"),
        "server".into(),
        "--single-node".into(),
    ]
}

/// The secrets are files beside it and never in it, nor in the environment
/// or the command line: `ctr containers info` prints all three.
///
/// SQLite for the metadata because in WAL mode it survives being killed,
/// which a container under a ceiling sometimes is.
///
/// With a domain, a bucket is also found by the name it is asked at, as
/// `<bucket>.<domain>`, on both doors; the path still works on either.
pub fn config(memory_limit: u64, config_mount: &str, domain: Option<&str>) -> String {
    let root = |section: &str| match domain {
        Some(domain) => format!("{section}root_domain = \".{domain}\"\n"),
        None => section.to_string(),
    };
    format!(
        "metadata_dir = \"{DATA_MOUNT}/{METADATA_DIR}\"\n\
         data_dir = \"{DATA_MOUNT}/data\"\n\
         metadata_snapshots_dir = \"{DATA_MOUNT}/{SNAPSHOTS_DIR}\"\n\
         db_engine = \"sqlite\"\n\
         replication_factor = 1\n\
         rpc_bind_addr = \"0.0.0.0:{RPC_PORT}\"\n\
         rpc_secret_file = \"{config_mount}/{RPC_SECRET_FILE}\"\n\
         block_ram_buffer_max = \"{}MiB\"\n\
         \n\
         {}\
         s3_region = \"{REGION}\"\n\
         api_bind_addr = \"0.0.0.0:{S3_PORT}\"\n\
         \n\
         {}\
         bind_addr = \"0.0.0.0:{WEB_PORT}\"\n\
         \n\
         [admin]\n\
         api_bind_addr = \"0.0.0.0:{ADMIN_PORT}\"\n\
         admin_token_file = \"{config_mount}/{ADMIN_TOKEN_FILE}\"\n",
        block_buffer(memory_limit) / MB,
        root("[s3_api]\n"),
        root("[s3_web]\n"),
    )
}

/// Garage's RPC reads its secret as 32 bytes of hex.
pub const NOT_AN_ALIAS: &str = "Garage finds a public bucket by the name it is read at, and \
                                 holds that name as one of the bucket's: 3 to 63 lowercase \
                                 letters, digits, dots and hyphens, and not an address";

/// Garage's rule for a bucket's name, which a public bucket's hostname has to
/// pass because it becomes one.
pub fn is_alias(name: &str) -> bool {
    (3..=63).contains(&name.len())
        && name
            .chars()
            .all(|c| matches!(c, '.' | '-' | 'a'..='z' | '0'..='9'))
        && !name.starts_with(['-', '.'])
        && !name.ends_with(['-', '.'])
        && name.parse::<std::net::IpAddr>().is_err()
        && !name.starts_with("xn--")
        && !name.contains(".xn--")
        && !name.ends_with("-s3alias")
}

pub fn is_rpc_secret(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Permissions {
    pub read: bool,
    pub write: bool,
    pub owner: bool,
}

/// Never owner: an owner can change CORS and lifecycle through S3, and the
/// next convergence would overwrite what it wrote.
pub fn permissions(access: Access) -> Permissions {
    Permissions {
        read: true,
        write: access.can_write(),
        owner: false,
    }
}

/// What a key holds and its row does not grant, which is what convergence
/// takes away.
pub fn excess(held: Permissions, wanted: Permissions) -> Permissions {
    Permissions {
        read: held.read && !wanted.read,
        write: held.write && !wanted.write,
        owner: held.owner && !wanted.owner,
    }
}

impl Permissions {
    pub fn any(self) -> bool {
        self.read || self.write || self.owner
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::policies::presets;

    const BLOCK_SIZE: u64 = MB;

    const WRITES_PER_REQUEST: u64 = 3;

    fn rungs() -> impl Iterator<Item = u64> {
        presets::LADDER.into_iter()
    }

    #[test]
    fn the_buffer_never_asks_for_more_than_garage_would_by_itself() {
        for rung in rungs().chain([64 * 1024 * MB]) {
            assert!(block_buffer(rung) <= DEFAULT_BLOCK_BUFFER, "{}", rung / MB);
        }
    }

    #[test]
    fn the_buffer_holds_every_block_one_request_writes_at_once() {
        for rung in rungs() {
            assert!(
                block_buffer(rung) >= 2 * WRITES_PER_REQUEST * BLOCK_SIZE,
                "{}",
                rung / MB
            );
        }
    }

    #[test]
    fn the_buffer_leaves_most_of_the_ceiling_to_the_rest_of_the_process() {
        for rung in rungs().filter(|rung| *rung >= FLOOR) {
            assert!(block_buffer(rung) <= rung / 8, "{}", rung / MB);
        }
    }

    #[test]
    fn under_the_floor_a_store_is_sized_as_the_floor() {
        assert_eq!(block_buffer(0), block_buffer(FLOOR));
        assert_eq!(block_buffer(64 * MB), block_buffer(FLOOR));
    }

    #[test]
    fn the_buffer_is_written_in_whole_mebibytes() {
        for limit in [FLOOR, 300 * MB, 1000 * MB + 12345] {
            assert_eq!(block_buffer(limit) % MB, 0, "{limit}");
        }
        assert!(config(300 * MB, "/etc/wabot", None).contains("block_ram_buffer_max = \"37MiB\""));
    }

    #[test]
    fn the_configuration_names_where_each_secret_is_and_holds_none() {
        let config = config(FLOOR, "/etc/wabot", None);
        assert!(config.contains("rpc_secret_file = \"/etc/wabot/rpc_secret\""));
        assert!(config.contains("admin_token_file = \"/etc/wabot/admin_token\""));
        assert!(!config.contains("rpc_secret ="));
        assert!(!config.contains("admin_token ="));
    }

    #[test]
    fn everything_garage_keeps_is_inside_the_volume_a_backup_copies() {
        let config = config(FLOOR, "/etc/wabot", None);
        for key in ["metadata_dir", "data_dir", "metadata_snapshots_dir"] {
            let line = config
                .lines()
                .find(|line| line.starts_with(&format!("{key} =")))
                .unwrap_or_else(|| panic!("{key} missing"));
            assert!(line.contains(&format!("\"{DATA_MOUNT}/")), "{line}");
        }
    }

    #[test]
    fn each_door_listens_where_the_node_and_the_edge_reach_it() {
        let config = config(FLOOR, "/etc/wabot", None);
        for port in [S3_PORT, RPC_PORT, WEB_PORT, ADMIN_PORT] {
            assert!(config.contains(&format!("\"0.0.0.0:{port}\"")), "{port}");
        }
        assert!(config.contains(&format!("s3_region = \"{REGION}\"")));
    }

    #[test]
    fn a_store_with_a_domain_finds_a_bucket_by_its_name_on_both_doors() {
        let config = config(FLOOR, "/etc/wabot", Some("files.shop.example.com"));
        let sections: Vec<&str> = config.split("\n\n").collect();
        for section in ["[s3_api]", "[s3_web]"] {
            let body = sections
                .iter()
                .find(|body| body.starts_with(section))
                .unwrap_or_else(|| panic!("{section} missing"));
            assert!(
                body.contains("root_domain = \".files.shop.example.com\""),
                "{body}"
            );
        }
        assert!(!super::config(FLOOR, "/etc/wabot", None).contains("root_domain"));
    }

    #[test]
    fn the_command_reads_the_file_the_node_wrote_and_lays_out_one_node() {
        assert_eq!(
            command("/etc/wabot"),
            [
                "/garage",
                "-c",
                "/etc/wabot/garage.toml",
                "server",
                "--single-node"
            ]
        );
    }

    #[test]
    fn a_series_is_served_by_the_release_this_node_was_built_with() {
        assert_eq!(
            image_for(DEFAULT_VERSION).as_deref(),
            Some("docker.io/dxflrs/garage:v2.5.0")
        );
        assert_eq!(image_for("1"), None);
    }

    #[test]
    fn a_secret_is_what_garage_reads_as_32_bytes() {
        assert!(is_rpc_secret(&"ab".repeat(32)));
        assert!(!is_rpc_secret(&"ab".repeat(31)));
        assert!(!is_rpc_secret(&"zz".repeat(32)));
    }

    #[test]
    fn a_tenant_key_reads_writes_only_if_asked_and_is_never_owner() {
        let read_write = permissions(Access::ReadWrite);
        let read_only = permissions(Access::ReadOnly);
        assert!(read_write.read && read_write.write && !read_write.owner);
        assert!(read_only.read && !read_only.write && !read_only.owner);
    }

    #[test]
    fn reclaiming_starts_below_the_ceiling_and_never_at_nothing() {
        for rung in rungs() {
            let high = memory_high(rung);
            assert!(high < rung.max(FLOOR), "{}", rung / MB);
            assert!(high >= rung.max(FLOOR) / 2, "{}", rung / MB);
        }
    }

    fn bucket() -> Bucket {
        Bucket::new(
            "b1".into(),
            "s1".into(),
            crate::domain::values::bucket_name::BucketName::parse("uploads").expect("a name"),
            5 * 1024 * MB,
        )
        .expect("a bucket")
    }

    #[test]
    fn a_new_bucket_tells_garage_its_quota_and_to_sweep_abandoned_uploads_and_nothing_else() {
        let settings = bucket_settings(&bucket());
        assert_eq!(settings["quotas"]["maxSize"], 5 * 1024 * MB);
        assert_eq!(settings["quotas"]["maxObjects"], serde_json::Value::Null);
        assert_eq!(
            settings["websiteAccess"],
            serde_json::json!({ "enabled": false })
        );
        assert_eq!(settings["corsRules"], serde_json::json!([]));
        assert_eq!(
            settings["lifecycleRules"],
            serde_json::json!([{
                "ID": "wabot",
                "Status": "Enabled",
                "AbortIncompleteMultipartUpload": { "DaysAfterInitiation": 7 },
            }])
        );
    }

    #[test]
    fn clearing_a_setting_is_said_to_garage_as_an_empty_list_not_left_out() {
        let mut bucket = bucket();
        bucket.set_expiry(None, None).expect("days");
        let settings = bucket_settings(&bucket);
        for field in ["corsRules", "lifecycleRules"] {
            assert_eq!(settings[field], serde_json::json!([]), "{field}");
        }
    }

    #[test]
    fn a_browser_may_upload_in_parts_from_each_listed_origin() {
        let mut bucket = bucket();
        bucket.cors_origins =
            crate::domain::values::cors_origin::CorsOrigin::parse_lines("https://app.example.com")
                .expect("origins");
        let rule = &bucket_settings(&bucket)["corsRules"][0];
        assert_eq!(
            rule["AllowedOrigin"],
            serde_json::json!(["https://app.example.com"])
        );
        assert_eq!(rule["ExposeHeader"], serde_json::json!(["ETag"]));
        for method in ["PUT", "POST", "DELETE"] {
            assert!(rule["AllowedMethod"]
                .as_array()
                .expect("methods")
                .contains(&serde_json::json!(method)));
        }
    }

    #[test]
    fn a_public_name_garage_could_not_hold_as_an_alias_is_not_one() {
        assert!(is_alias("uploads.files.shop.example.com"));
        assert!(is_alias("assets.example.com"));
        assert!(!is_alias(&format!("{}.example.com", "a".repeat(60))));
        assert!(!is_alias("10.0.0.1"));
        assert!(!is_alias("xn--bcher-kva.example"));
        assert!(!is_alias("Assets.example.com"));
    }

    #[test]
    fn a_public_bucket_serves_an_index_and_expiry_deletes_by_age() {
        let mut bucket = bucket();
        bucket.public = true;
        bucket.set_expiry(Some(30), None).expect("days");
        let settings = bucket_settings(&bucket);
        assert_eq!(
            settings["websiteAccess"],
            serde_json::json!({ "enabled": true, "indexDocument": "index.html" })
        );
        assert_eq!(
            settings["lifecycleRules"][0]["Expiration"],
            serde_json::json!({ "Days": 30 })
        );
        assert!(settings["lifecycleRules"][0]
            .get("AbortIncompleteMultipartUpload")
            .is_none());
    }

    #[test]
    fn a_key_made_owner_elsewhere_loses_it_and_keeps_what_its_row_grants() {
        let held = Permissions {
            read: true,
            write: true,
            owner: true,
        };
        let taken = excess(held, permissions(Access::ReadOnly));
        assert_eq!(
            taken,
            Permissions {
                read: false,
                write: true,
                owner: true
            }
        );
        assert!(!excess(
            permissions(Access::ReadWrite),
            permissions(Access::ReadWrite)
        )
        .any());
    }
}
