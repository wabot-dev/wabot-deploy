//! What the node is and where it keeps things.
//!
//! A TOML file rather than environment variables, which is the
//! opposite of what the framework does and deliberate. `RestServerConfig`
//! reads `PORT` because a twelve-factor service is configured by
//! whatever started it; this is a **system daemon** an operator edits,
//! and structured configuration in a systemd `EnvironmentFile` is a
//! flat list of shouting.
//!
//! Environment variables still win where they are set, so a container
//! or a test can override one value without a file.
//!
//! ## Unknown keys are an error
//!
//! `deny_unknown_fields` throughout. A typo in a config file otherwise
//! means the node runs with a default the operator believes they
//! changed — the worst kind of failure, because everything looks fine.
//! The cost is that a section for a feature that has not shipped yet
//! is refused, which is honest: it would have been ignored anyway.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where `install` writes the file, and where `serve` looks for it.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/wabot-deploy/config.toml";

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{path} is not valid configuration: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

pub type ConfigResult<T> = Result<T, ConfigError>;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub node: NodeConfig,
    #[serde(default)]
    pub edge: EdgeConfig,
    #[serde(default)]
    pub acme: AcmeConfig,
    #[serde(default)]
    pub log: LogConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    /// The hostname this node answers to.
    ///
    /// Optional on purpose: without one the node still serves, on a
    /// self-signed certificate, so it can be installed and inspected
    /// before DNS exists. ACME needs it, and turns on when it appears.
    #[serde(default)]
    pub domain: Option<String>,
    /// Database, certificates, and anything else with state.
    #[serde(default = "default_data_dir")]
    pub data_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeConfig {
    #[serde(default = "default_https_port")]
    pub https_port: u16,
    /// Needed even when everything is HTTPS: the ACME HTTP-01
    /// challenge arrives here, and so does every visitor who typed the
    /// hostname without a scheme.
    #[serde(default = "default_http_port")]
    pub http_port: u16,
}

/// Which certificate authority to ask, and as whom.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcmeConfig {
    /// `production` (the default), `staging`, or a directory URL.
    ///
    /// Production is the default because that is what a real node
    /// wants and a wrong default here means a certificate no browser
    /// trusts. Staging exists for testing, and matters: production
    /// refuses more than five failed orders per account per hour, so
    /// debugging a DNS problem against it locks you out for the rest
    /// of the hour.
    #[serde(default = "default_acme_directory")]
    pub directory: String,
    /// Contact address. The CA mails it before a certificate expires,
    /// which is the last warning before an outage.
    #[serde(default)]
    pub email: Option<String>,
    /// Turn ACME off and keep serving the local authority's
    /// certificate — for a node reachable only on a private network,
    /// where no public CA can validate anything.
    #[serde(default)]
    pub disabled: bool,
}

impl AcmeConfig {
    /// The directory URL to talk to.
    pub fn directory_url(&self) -> &str {
        match self.directory.as_str() {
            "production" | "prod" => "https://acme-v02.api.letsencrypt.org/directory",
            "staging" | "test" => "https://acme-staging-v02.api.letsencrypt.org/directory",
            url => url,
        }
    }

    /// Whether this is Let's Encrypt's staging environment, whose
    /// certificates are untrusted by design — worth saying out loud
    /// rather than leaving somebody to wonder why the browser still
    /// complains.
    pub fn is_staging(&self) -> bool {
        self.directory_url().contains("staging")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogConfig {
    /// A `tracing` filter, e.g. `info,wabot=debug`.
    #[serde(default = "default_log_filter")]
    pub filter: String,
}

fn default_data_dir() -> PathBuf {
    PathBuf::from("/var/lib/wabot-deploy")
}
fn default_https_port() -> u16 {
    443
}
fn default_http_port() -> u16 {
    80
}
fn default_log_filter() -> String {
    "info".to_string()
}
fn default_acme_directory() -> String {
    "production".to_string()
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            domain: None,
            data_dir: default_data_dir(),
        }
    }
}

impl Default for EdgeConfig {
    fn default() -> Self {
        Self {
            https_port: default_https_port(),
            http_port: default_http_port(),
        }
    }
}

impl Default for AcmeConfig {
    fn default() -> Self {
        Self {
            directory: default_acme_directory(),
            email: None,
            disabled: false,
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: default_log_filter(),
        }
    }
}

impl Config {
    /// Read `path`, then let the environment override.
    ///
    /// A missing file is **not** an error: the defaults describe a
    /// working node, and `serve` before `install` should complain
    /// about the database it cannot open, not about a file it could
    /// have done without.
    pub fn load(path: &Path) -> ConfigResult<Self> {
        let mut config = match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|source| ConfigError::Parse {
                path: path.to_path_buf(),
                source,
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(source) => {
                return Err(ConfigError::Read {
                    path: path.to_path_buf(),
                    source,
                })
            }
        };
        config.apply_env()?;
        config.validate()?;
        Ok(config)
    }

    /// Overrides for the handful of values worth setting without a
    /// file — a container, a test, a one-off run.
    fn apply_env(&mut self) -> ConfigResult<()> {
        if let Some(domain) = env_string("WABOT_DEPLOY_DOMAIN") {
            self.node.domain = Some(domain);
        }
        if let Some(dir) = env_string("WABOT_DEPLOY_DATA_DIR") {
            self.node.data_dir = PathBuf::from(dir);
        }
        if let Some(port) = env_port("WABOT_DEPLOY_HTTPS_PORT")? {
            self.edge.https_port = port;
        }
        if let Some(port) = env_port("WABOT_DEPLOY_HTTP_PORT")? {
            self.edge.http_port = port;
        }
        if let Some(directory) = env_string("WABOT_DEPLOY_ACME_DIRECTORY") {
            self.acme.directory = directory;
        }
        if let Some(email) = env_string("WABOT_DEPLOY_ACME_EMAIL") {
            self.acme.email = Some(email);
        }
        // `RUST_LOG` too, because every Rust operator reaches for it
        // first and a daemon that ignored it would be quietly annoying.
        if let Some(filter) = env_string("WABOT_DEPLOY_LOG").or_else(|| env_string("RUST_LOG")) {
            self.log.filter = filter;
        }
        Ok(())
    }

    fn validate(&self) -> ConfigResult<()> {
        if self.edge.https_port == self.edge.http_port {
            return Err(ConfigError::Invalid(format!(
                "edge.https_port and edge.http_port are both {} — they are two listeners",
                self.edge.https_port
            )));
        }
        if let Some(domain) = &self.node.domain {
            let domain = domain.trim();
            if domain.is_empty() {
                return Err(ConfigError::Invalid(
                    "node.domain is set but empty — remove the key instead".into(),
                ));
            }
            // Not a hostname validator: just the mistakes that would
            // otherwise surface much later as a baffling ACME failure.
            if domain.contains("://") || domain.contains('/') {
                return Err(ConfigError::Invalid(format!(
                    "node.domain should be a hostname, not a URL: {domain:?}"
                )));
            }
        }
        Ok(())
    }

    /// The database file. One place decides, so `install`, `serve` and
    /// `doctor` cannot disagree about where the node's state lives.
    pub fn database_path(&self) -> PathBuf {
        self.node.data_dir.join("db").join("node.db")
    }

    pub fn certificates_dir(&self) -> PathBuf {
        self.node.data_dir.join("certs")
    }

    /// Serialize for `install` to write.
    pub fn to_toml(&self) -> String {
        let body = toml::to_string_pretty(self).expect("config is serializable");
        format!(
            "# wabot-deploy node configuration.\n\
             #\n\
             # Written by `wabot-deploy install`; edit freely, it is never\n\
             # rewritten. Unknown keys are refused rather than ignored, so a\n\
             # typo fails loudly instead of leaving a default in place.\n\
             #\n\
             # node.domain seeds the name this node answers to. After the\n\
             # first start it lives in the database, where the console can\n\
             # change it — editing it here again does nothing unless you\n\
             # re-run `install --domain`.\n\n{body}"
        )
    }

    /// Write the file, refusing to clobber one that exists.
    ///
    /// Idempotence for the install step: re-running must converge, and
    /// silently replacing an operator's edits is the opposite of that.
    pub fn write_if_absent(&self, path: &Path) -> ConfigResult<bool> {
        if path.exists() {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        std::fs::write(path, self.to_toml()).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(true)
    }
}

fn env_string(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

fn env_port(name: &str) -> ConfigResult<Option<u16>> {
    match env_string(name) {
        None => Ok(None),
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| ConfigError::Invalid(format!("{name} is not a port number: {value:?}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).expect("write");
        (dir, path)
    }

    #[test]
    fn a_missing_file_is_the_defaults() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = Config::load(&dir.path().join("absent.toml")).expect("load");
        assert_eq!(config.edge.https_port, 443);
        assert!(config.node.domain.is_none());
    }

    #[test]
    fn a_partial_file_keeps_the_defaults_for_the_rest() {
        let (_dir, path) = write("[node]\ndomain = \"node.example.com\"\n");
        let config = Config::load(&path).expect("load");
        assert_eq!(config.node.domain.as_deref(), Some("node.example.com"));
        assert_eq!(
            config.edge.http_port, 80,
            "untouched sections still default"
        );
        assert_eq!(config.node.data_dir, PathBuf::from("/var/lib/wabot-deploy"));
    }

    /// The reason for `deny_unknown_fields`: a typo must not leave the
    /// operator believing they changed something.
    #[test]
    fn a_misspelled_key_is_refused() {
        let (_dir, path) = write("[node]\ndomian = \"typo.example.com\"\n");
        let error = Config::load(&path).expect_err("refused");
        assert!(matches!(error, ConfigError::Parse { .. }), "{error}");
        assert!(error.to_string().contains("domian"), "{error}");
    }

    #[test]
    fn two_listeners_cannot_share_a_port() {
        let (_dir, path) = write("[edge]\nhttps_port = 8080\nhttp_port = 8080\n");
        let error = Config::load(&path).expect_err("refused");
        assert!(error.to_string().contains("two listeners"), "{error}");
    }

    #[test]
    fn a_url_where_a_hostname_belongs_is_refused() {
        let (_dir, path) = write("[node]\ndomain = \"https://node.example.com\"\n");
        let error = Config::load(&path).expect_err("refused");
        assert!(error.to_string().contains("hostname"), "{error}");
    }

    /// Round-tripping matters because `install` writes this file and a
    /// later run reads it back: a value that serializes to something
    /// the parser rejects would break the second install, not the
    /// first.
    #[test]
    fn what_install_writes_is_what_load_reads() {
        let mut original = Config::default();
        original.node.domain = Some("node.example.com".into());
        original.edge.https_port = 8443;

        let (_dir, path) = write(&original.to_toml());
        let reloaded = Config::load(&path).expect("load");

        assert_eq!(reloaded.node.domain, original.node.domain);
        assert_eq!(reloaded.edge.https_port, 8443);
        assert_eq!(reloaded.node.data_dir, original.node.data_dir);
    }

    #[test]
    fn writing_never_clobbers_an_existing_file() {
        let (_dir, path) = write("[node]\ndomain = \"kept.example.com\"\n");
        assert!(
            !Config::default().write_if_absent(&path).expect("write"),
            "an existing file is left alone"
        );
        assert_eq!(
            Config::load(&path).expect("load").node.domain.as_deref(),
            Some("kept.example.com")
        );
    }

    #[test]
    fn the_database_lives_under_the_data_dir() {
        let mut config = Config::default();
        config.node.data_dir = PathBuf::from("/srv/node");
        assert_eq!(
            config.database_path(),
            PathBuf::from("/srv/node/db/node.db")
        );
    }
}
