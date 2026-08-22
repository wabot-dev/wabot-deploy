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
    pub overlay: OverlayConfig,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub backup: BackupConfig,
    /// Where this was read from, for the one thing that has to write a
    /// value *back*: the bucket credential the console stores.
    ///
    /// Skipped by serde in both directions. It is not a setting — a
    /// config file naming its own path is a fact that goes stale the
    /// first time somebody copies it — and [`Config::load`] is the only
    /// thing that fills it in. `None` is a config nobody read from a
    /// file: a default, a test, an environment-only run.
    #[serde(skip)]
    pub source: Option<PathBuf>,
}

/// Where `backup --out s3://…` gets its credentials.
///
/// **In the config file and not in the database, on purpose.** A
/// credential that can read every backup in the network is precisely the
/// thing that must not be inside the thing being backed up: restore a
/// node from a copy that carried it, and you have restored that node's
/// ability to reach every other node's backups. A backup holds `node.db`,
/// the volumes and the images; it does not hold `/etc`.
///
/// **The console writes this section**, so configuring a bucket is not an
/// SSH session — see `config::store_s3` and the Backup tab. What keeps
/// the property above is *which file* it writes, not who writes it: the
/// console is already root on this machine, and the secret still lands
/// somewhere no backup carries.
///
/// And it is read back **at the moment of use**, never from a `Config`
/// cloned at startup — see [`Config::s3`]. Otherwise a credential saved
/// from the console would not apply until somebody restarted the node,
/// which is the sort of thing an operator discovers at the second failed
/// backup.
///
/// Absent is the ordinary case. A node with no section here can still
/// back up to a path or over SSH — SSH uses the operator's own keys and
/// needs nothing stored.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupConfig {
    #[serde(default)]
    pub s3: Option<crate::commands::s3::Credentials>,
}

/// The private network between nodes.
///
/// One setting, because there is one decision: which UDP port this
/// node's WireGuard listens on. Everything else about the overlay — the
/// addresses, the keys, the peers — is allocated by whoever enrolled
/// whom and lives in the database, where a config file cannot make two
/// nodes disagree about it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayConfig {
    /// WireGuard's assigned port. A node behind NAT never needs this
    /// opened — it dials outbound — but a node that enrols others does,
    /// because that is where their handshakes arrive.
    #[serde(default = "default_overlay_port")]
    pub port: u16,
}

fn default_overlay_port() -> u16 {
    crate::network::tunnel::DEFAULT_PORT
}

impl Default for OverlayConfig {
    fn default() -> Self {
        Self {
            port: default_overlay_port(),
        }
    }
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
        config.source = Some(path.to_path_buf());
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
        // Worth an override of its own rather than only a file key: a
        // node run on a laptop has no name a CA can validate, and the
        // moment somebody sets a domain in its console the renewal loop
        // starts placing real orders. Production refuses more than five
        // failed ones an hour, so the cost of that mistake is an hour
        // of not being able to ask for the certificate you meant.
        if let Some(disabled) = env_flag("WABOT_DEPLOY_ACME_DISABLED")? {
            self.acme.disabled = disabled;
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

    /// The bucket credential as the file has it **now**.
    ///
    /// Read from disk rather than taken from this struct, because the
    /// console can write it while the node runs: a copy made at startup
    /// would say "no credentials" to the backup taken a minute after
    /// somebody saved some. One read per backup, which is nothing beside
    /// what a backup does.
    ///
    /// Falls back to what was loaded when there is no file to read — a
    /// default `Config`, a test, an environment-only run — so this is
    /// always the same answer the rest of the code would have given.
    pub fn s3(&self) -> Option<crate::commands::s3::Credentials> {
        let Some(path) = &self.source else {
            return self.backup.s3.clone();
        };
        // A struct of its own rather than parsing the whole `Config`:
        // this must not fail because some *other* section of the file is
        // newer than this binary, and `deny_unknown_fields` on `Config`
        // would make it.
        #[derive(Deserialize, Default)]
        struct JustBackup {
            #[serde(default)]
            backup: BackupConfig,
        }
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<JustBackup>(&text) {
                Ok(read) => read.backup.s3,
                Err(error) => {
                    tracing::warn!(%error, path = %path.display(), "could not read the bucket credential");
                    self.backup.s3.clone()
                }
            },
            Err(_) => self.backup.s3.clone(),
        }
    }

    /// Serialize for `install` to write.
    pub fn to_toml(&self) -> String {
        let body = toml::to_string_pretty(self).expect("config is serializable");
        format!(
            "# wabot-deploy node configuration.\n\
             #\n\
             # Written by `wabot-deploy install`; edit freely. The one thing\n\
             # that rewrites anything here is the console's Backup tab, and\n\
             # only the [backup.s3] section — the rest of this file, comments\n\
             # and all, is left exactly as you leave it. Unknown keys are\n\
             # refused rather than ignored, so a typo fails loudly instead of\n\
             # leaving a default in place.\n\
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

/// Write the `[backup.s3]` section, or take it out.
///
/// **Text surgery, not a re-serialization of the whole file.** Round-
/// tripping through `toml::to_string_pretty` would produce valid
/// configuration and throw away every comment and every bit of ordering
/// the operator put there — and the file's own header says "edit
/// freely". So this replaces exactly the lines from `[backup.s3]` to the
/// next section header, and leaves the rest byte for byte.
///
/// **Written through a temporary file and renamed.** A truncated
/// `config.toml` is a node that will not start: `load` refuses a file it
/// cannot parse, which is the right behaviour and the wrong moment. A
/// rename is atomic on the same filesystem, so the file is either the old
/// one or the new one.
///
/// **And 0600.** The doc on [`BackupConfig`] calls this file root-only,
/// and now that the console puts a secret in it that had better be true
/// rather than whatever the umask was at install.
pub fn store_s3(
    path: &Path,
    credentials: Option<&crate::commands::s3::Credentials>,
) -> ConfigResult<()> {
    let existing = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            })
        }
    };

    let updated = replace_section(&existing, "[backup.s3]", credentials.map(section_for));
    // Parsed before it is written. A section this built and the file
    // could not read back is a node that does not come up after the next
    // restart, discovered by somebody who only clicked Save.
    toml::from_str::<toml::Table>(&updated).map_err(|source| ConfigError::Parse {
        path: path.to_path_buf(),
        source,
    })?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let temporary = path.with_extension("toml.saving");
    std::fs::write(&temporary, &updated).map_err(|source| ConfigError::Write {
        path: temporary.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&temporary, path).map_err(|source| ConfigError::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// The section as it goes into the file.
///
/// The comment goes with it: somebody reading this file at two in the
/// morning needs to know that these lines are written by something, and
/// where from.
fn section_for(credentials: &crate::commands::s3::Credentials) -> String {
    let mut section = String::from(
        "[backup.s3]
         # Written by the console's Backup tab. Editing it here works too —
         # it is read at the moment of use, so no restart is needed either
         # way. It is deliberately not in the database: a key that can read
         # every backup in the network must not be inside the thing being
         # backed up.
",
    );
    section.push_str(&format!(
        "access_key_id = {}
",
        toml_string(&credentials.access_key_id)
    ));
    section.push_str(&format!(
        "secret_access_key = {}
",
        toml_string(&credentials.secret_access_key)
    ));
    section.push_str(&format!(
        "region = {}
",
        toml_string(&credentials.region)
    ));
    if let Some(endpoint) = &credentials.endpoint {
        section.push_str(&format!(
            "endpoint = {}
",
            toml_string(endpoint)
        ));
    }
    section
}

/// A TOML basic string, escaped.
///
/// A secret is whatever the provider generated, and some of them contain
/// characters — `\`, `"` — that would end the literal early and produce a
/// file that does not parse or, worse, one that parses to the wrong
/// value.
fn toml_string(value: &str) -> String {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Replace one section of a TOML file, or add it, or drop it.
///
/// A section runs from its header to the next line that starts one. Pure,
/// so the awkward cases — no trailing newline, the section last in the
/// file, a header with a comment after it — are a test rather than a
/// thing somebody discovers on a node.
fn replace_section(text: &str, header: &str, body: Option<String>) -> String {
    let mut out = String::new();
    let mut lines = text.lines();
    let mut replaced = false;

    while let Some(line) = lines.next() {
        if line.trim() != header {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        // Found it: skip to the next header, and put the new body here so
        // the section keeps the place the operator's file gave it.
        if let Some(body) = &body {
            out.push_str(body);
        }
        replaced = true;
        for rest in lines.by_ref() {
            if rest.trim_start().starts_with('[') {
                out.push_str(rest);
                out.push('\n');
                break;
            }
        }
    }

    if !replaced {
        if let Some(body) = &body {
            if !out.is_empty() && !out.ends_with("\n\n") {
                out.push('\n');
            }
            out.push_str(body);
        }
    }
    out
}

fn env_string(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Some(value),
        _ => None,
    }
}

/// A boolean from the environment, refusing what it cannot read.
///
/// Not "anything non-empty is true": `WABOT_DEPLOY_ACME_DISABLED=no`
/// reads as off to everyone who types it, and silently meaning on is
/// the same failure `deny_unknown_fields` exists to prevent.
fn env_flag(name: &str) -> ConfigResult<Option<bool>> {
    match env_string(name) {
        None => Ok(None),
        Some(value) => parse_flag(name, &value).map(Some),
    }
}

/// Split out from [`env_flag`] so it can be tested without setting a
/// variable: the suite runs in threads, and a test that mutates the
/// environment is a test that fails whichever other one reads it.
fn parse_flag(name: &str, value: &str) -> ConfigResult<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(ConfigError::Invalid(format!(
            "{name} is not a yes or a no: {value:?}"
        ))),
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

    fn some_credentials() -> crate::commands::s3::Credentials {
        crate::commands::s3::Credentials {
            access_key_id: "AKIAEXAMPLE".into(),
            secret_access_key: "a-secret/with\\slashes".into(),
            region: "us-east-1".into(),
            endpoint: Some("https://s3.us-west-004.backblazeb2.com".into()),
        }
    }

    /// The console has to be able to configure a bucket, and the rest of
    /// the operator's file has to survive it.
    ///
    /// Jorge asked for the mechanism: configuring backups to a bucket was
    /// an SSH session and a text editor, which is the shape of a feature
    /// nobody sets up. The file is still where it goes — a key that can
    /// read every backup in the network must not be inside the thing
    /// being backed up.
    #[test]
    fn a_credential_is_written_without_disturbing_the_file_around_it() {
        let (_dir, path) = write(
            "# my own notes\n\
             [node]\n\
             domain = \"node.example.com\"   # and a comment on the line\n\n\
             [log]\n\
             filter = \"info\"\n",
        );

        store_s3(&path, Some(&some_credentials())).expect("store");
        let text = std::fs::read_to_string(&path).expect("read it back");

        assert!(text.contains("# my own notes"), "{text}");
        assert!(
            text.contains("domain = \"node.example.com\"   # and a comment on the line"),
            "the operator's line is untouched: {text}"
        );
        assert!(text.contains("[log]"), "{text}");

        // And it is configuration this binary can read — which is the
        // claim that matters, because a file it cannot parse is a node
        // that does not come back from its next restart.
        let config = Config::load(&path).expect("load what was written");
        let stored = config.s3().expect("a credential");
        assert_eq!(stored.access_key_id, "AKIAEXAMPLE");
        assert_eq!(stored.secret_access_key, "a-secret/with\\slashes");
        assert_eq!(
            stored.endpoint.as_deref(),
            Some("https://s3.us-west-004.backblazeb2.com")
        );
    }

    /// Saved twice is one section, not two.
    ///
    /// Two `[backup.s3]` headers is a file TOML refuses outright, so an
    /// operator who changed their mind about a region would have taken
    /// the node down at its next restart.
    #[test]
    fn saving_again_replaces_the_section_it_wrote() {
        let (_dir, path) = write("[node]\ndomain = \"a.example\"\n");
        store_s3(&path, Some(&some_credentials())).expect("first");

        let mut second = some_credentials();
        second.region = "eu-central-1".into();
        second.endpoint = None;
        store_s3(&path, Some(&second)).expect("second");

        let text = std::fs::read_to_string(&path).expect("read");
        assert_eq!(text.matches("[backup.s3]").count(), 1, "{text}");
        let config = Config::load(&path).expect("load");
        let stored = config.s3().expect("a credential");
        assert_eq!(stored.region, "eu-central-1");
        assert_eq!(
            stored.endpoint, None,
            "an endpoint that was cleared: {text}"
        );
    }

    /// Forgetting it takes the section out and leaves the file working.
    #[test]
    fn forgetting_the_credential_removes_only_that_section() {
        let (_dir, path) = write("[node]\ndomain = \"a.example\"\n\n[log]\nfilter = \"warn\"\n");
        store_s3(&path, Some(&some_credentials())).expect("store");
        store_s3(&path, None).expect("forget");

        let text = std::fs::read_to_string(&path).expect("read");
        assert!(!text.contains("backup.s3"), "{text}");
        assert!(!text.contains("AKIAEXAMPLE"), "and not the key: {text}");
        let config = Config::load(&path).expect("load");
        assert_eq!(config.node.domain.as_deref(), Some("a.example"));
        assert_eq!(config.log.filter, "warn");
        assert!(config.s3().is_none());
    }

    /// The file is read at the moment of use, not once at startup.
    ///
    /// This is what makes the console's Save apply to the very next
    /// backup. A `Config` cloned when the node started would have said
    /// "no credentials" until somebody restarted it, which is the sort of
    /// thing an operator finds out about at the second failed backup.
    #[test]
    fn the_credential_comes_from_the_file_each_time() {
        let (_dir, path) = write("[node]\ndomain = \"a.example\"\n");
        let config = Config::load(&path).expect("load");
        assert!(config.s3().is_none());

        store_s3(&path, Some(&some_credentials())).expect("store");
        assert_eq!(
            config.s3().expect("read again").access_key_id,
            "AKIAEXAMPLE",
            "the same Config, and the file has moved on"
        );
    }

    /// Root-only, because the console now puts a secret in it.
    #[cfg(unix)]
    #[test]
    fn the_file_holding_a_secret_is_not_world_readable() {
        use std::os::unix::fs::PermissionsExt;
        let (_dir, path) = write("[node]\ndomain = \"a.example\"\n");
        store_s3(&path, Some(&some_credentials())).expect("store");

        let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
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

    /// The variable that keeps a laptop from placing real ACME orders.
    /// Reading `no` as "yes, disable it" would arm the thing it is
    /// there to disarm.
    #[test]
    fn a_flag_means_what_it_says_or_is_refused() {
        for yes in ["1", "true", "YES", " on "] {
            assert!(parse_flag("F", yes).expect("accepted"), "{yes:?}");
        }
        for no in ["0", "false", "NO", " off "] {
            assert!(!parse_flag("F", no).expect("accepted"), "{no:?}");
        }
        let error = parse_flag("F", "maybe").expect_err("refused");
        assert!(error.to_string().contains("maybe"), "{error}");
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
