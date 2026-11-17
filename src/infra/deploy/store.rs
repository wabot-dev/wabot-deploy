//! What a store's container is given: Garage's configuration and its two
//! secrets, written from the rows on every deployment.

use std::path::Path;

use crate::domain::entities::object_store::ObjectStore;
use crate::domain::policies::{garage, postgres};
use crate::infra::deploy::database::config_dir;
use crate::infra::runtime::spec::BindMount;

pub struct Prepared {
    pub command: Vec<String>,
    pub mount: BindMount,
    pub memory_high: u64,
}

pub fn prepare(
    data_dir: &Path,
    container_id: &str,
    store: &ObjectStore,
    memory_limit: Option<u64>,
) -> std::io::Result<Prepared> {
    if !garage::is_rpc_secret(&store.rpc_secret) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "the store's row holds no secret Garage would read, and it refuses to start without one",
        ));
    }
    let conf = config_dir(data_dir, container_id);
    std::fs::create_dir_all(&conf)?;
    let limit = memory_limit.unwrap_or(garage::FLOOR);
    std::fs::write(
        conf.join(garage::CONFIG_FILE),
        garage::config(limit, postgres::CONFIG_MOUNT),
    )?;
    secret(&conf.join(garage::RPC_SECRET_FILE), &store.rpc_secret)?;
    secret(&conf.join(garage::ADMIN_TOKEN_FILE), &store.admin_token)?;
    Ok(Prepared {
        command: garage::command(postgres::CONFIG_MOUNT),
        mount: BindMount {
            source: conf,
            destination: postgres::CONFIG_MOUNT.to_string(),
            read_only: true,
        },
        memory_high: garage::memory_high(limit),
    })
}

/// Created at 0600 rather than written and then narrowed: Garage refuses a
/// secret file anybody else could read, and so should the moment between.
fn secret(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ObjectStore {
        ObjectStore {
            service_id: "s1".into(),
            version: garage::DEFAULT_VERSION.into(),
            rpc_secret: "ab".repeat(32),
            admin_token: "admin-token-value".into(),
            restored_from: None,
        }
    }

    #[test]
    fn the_secrets_are_files_only_their_owner_reads_and_the_configuration_names_them() {
        let dir = tempfile::tempdir().expect("dir");
        let prepared = prepare(dir.path(), "demo.files", &store(), None).expect("prepared");
        let conf = config_dir(dir.path(), "demo.files");
        assert_eq!(prepared.mount.source, conf);
        assert!(prepared.mount.read_only);
        for (file, text) in [
            (garage::RPC_SECRET_FILE, "ab".repeat(32)),
            (garage::ADMIN_TOKEN_FILE, "admin-token-value".to_string()),
        ] {
            let path = conf.join(file);
            assert_eq!(std::fs::read_to_string(&path).expect("secret"), text);
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
                assert_eq!(mode & 0o077, 0, "{file} is readable by others: {mode:o}");
            }
        }
        let config = std::fs::read_to_string(conf.join(garage::CONFIG_FILE)).expect("config");
        assert!(!config.contains(&"ab".repeat(32)));
        assert!(!config.contains("admin-token-value"));
    }

    #[test]
    fn a_second_deployment_rewrites_a_secret_that_changed() {
        let dir = tempfile::tempdir().expect("dir");
        prepare(dir.path(), "demo.files", &store(), None).expect("first");
        let mut changed = store();
        changed.admin_token = "another".into();
        prepare(dir.path(), "demo.files", &changed, None).expect("second");
        let path = config_dir(dir.path(), "demo.files").join(garage::ADMIN_TOKEN_FILE);
        assert_eq!(std::fs::read_to_string(path).expect("token"), "another");
    }
}
