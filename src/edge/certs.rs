//! What certificate to present, and where it comes from before there
//! is a domain.
//!
//! ## The resolver is synchronous, and that shapes everything
//!
//! `rustls::server::ResolvesServerCert::resolve` cannot await. It
//! picks from what is already loaded, so a certificate has to exist
//! *before* the handshake that needs it. Issuance therefore happens
//! when a hostname is registered — never on demand inside a handshake.
//!
//! That is not only a technical constraint. On-demand issuance means
//! anyone who can send SNI can make the node ask a certificate
//! authority for something, which is a fine way to burn a rate limit
//! you did not choose to spend.
//!
//! ## A local CA, not a bare self-signed leaf
//!
//! The leaf is reissued whenever the node's names change — a domain
//! gets configured, an address moves. With a bare self-signed
//! certificate the operator re-trusts every time, which is how people
//! learn to click through warnings. With a CA they trust once.

use std::collections::HashMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use time::{Duration, OffsetDateTime};
use wabot::rest::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use wabot::rest::rustls::server::{ClientHello, ResolvesServerCert};
use wabot::rest::rustls::sign::CertifiedKey;
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

/// How long a locally-issued leaf lasts.
///
/// Deliberately short-ish: it is a placeholder until ACME, and a
/// self-signed certificate that outlives the reason for it is one
/// nobody notices is still in use.
const SELF_SIGNED_DAYS: i64 = 90;
const CA_YEARS: i64 = 10;
/// Reissue once a certificate is inside this window of expiring.
const RENEW_WITHIN_DAYS: i64 = 30;

/// The name a node answers to before it has a domain.
pub const FALLBACK_NAME: &str = "localhost";

#[derive(Debug, thiserror::Error)]
pub enum CertError {
    #[error("storage: {0}")]
    Storage(#[from] wabot::sqlite::SqliteError),
    #[error("generating a certificate: {0}")]
    Generate(#[from] rcgen::Error),
    #[error("{0}")]
    Invalid(String),
}

type CertResult<T> = Result<T, CertError>;

/// A certificate as it is stored and served.
#[derive(Debug, Clone)]
pub struct StoredCert {
    pub domain: String,
    /// Every name it covers, sorted. See the migration for why this is
    /// stored rather than read back out of the certificate.
    pub names: Vec<String>,
    pub cert_pem: String,
    pub key_pem: String,
    pub issuer: String,
    pub not_after: i64,
}

/// Sorted and deduplicated, so two requests for the same set compare
/// equal however they were spelled.
fn canonical(names: &[String]) -> Vec<String> {
    let mut names = names.to_vec();
    names.sort();
    names.dedup();
    names
}

/// Certificates by SNI name, swapped whole when one changes.
///
/// `ArcSwap` because `resolve` runs on every handshake and a lock
/// there would serialize connections behind whatever the renewal loop
/// is doing.
pub struct CertResolver {
    by_name: ArcSwap<HashMap<String, Arc<CertifiedKey>>>,
    /// Served when SNI names something unknown — or names nothing at
    /// all, which is what an IP-address client does.
    fallback: ArcSwap<Option<Arc<CertifiedKey>>>,
}

impl std::fmt::Debug for CertResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertResolver")
            .field("names", &self.names())
            .finish()
    }
}

impl Default for CertResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl CertResolver {
    pub fn new() -> Self {
        Self {
            by_name: ArcSwap::from_pointee(HashMap::new()),
            fallback: ArcSwap::from_pointee(None),
        }
    }

    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.by_name.load().keys().cloned().collect();
        names.sort();
        names
    }

    /// Replace the whole set. Called at boot and whenever a
    /// certificate is issued or renewed.
    pub fn replace(&self, certificates: &[StoredCert]) -> CertResult<()> {
        let mut by_name = HashMap::with_capacity(certificates.len());
        for stored in certificates {
            by_name.insert(stored.domain.clone(), Arc::new(certified_key(stored)?));
        }
        // Whatever we have for the fallback name, or any one
        // certificate — a handshake with no SNI must still get *an*
        // answer, or the operator sees a TLS error instead of a
        // warning they can click through.
        let fallback = by_name
            .get(FALLBACK_NAME)
            .or_else(|| by_name.values().next())
            .cloned();

        self.by_name.store(Arc::new(by_name));
        self.fallback.store(Arc::new(fallback));
        Ok(())
    }
}

impl ResolvesServerCert for CertResolver {
    fn resolve(&self, hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        if let Some(name) = hello.server_name() {
            if let Some(key) = self.by_name.load().get(name) {
                return Some(key.clone());
            }
            tracing::debug!(sni = %name, "no certificate for this name; serving the fallback");
        }
        self.fallback.load().as_ref().clone()
    }
}

/// Turn stored PEM into what rustls serves.
fn certified_key(stored: &StoredCert) -> CertResult<CertifiedKey> {
    let chain: Vec<CertificateDer<'static>> = pem_certificates(&stored.cert_pem)?;
    if chain.is_empty() {
        return Err(CertError::Invalid(format!(
            "no certificate found in the stored PEM for {}",
            stored.domain
        )));
    }
    let key = pem_private_key(&stored.key_pem)?;
    let signing = wabot::rest::rustls::crypto::ring::sign::any_supported_type(&key)
        .map_err(|error| CertError::Invalid(format!("unusable private key: {error}")))?;
    Ok(CertifiedKey::new(chain, signing))
}

/// A minimal PEM reader.
///
/// `rustls-pemfile` would do this, but it is a whole dependency for
/// two block types, and the framework does not re-export one — so
/// pulling it in risks a second copy of the `rustls-pki-types` that
/// `CertificateDer` comes from.
fn pem_blocks<'a>(pem: &'a str, label: &str) -> Vec<Vec<u8>> {
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut blocks = Vec::new();
    let mut rest: &'a str = pem;

    while let Some(start) = rest.find(&begin) {
        let after = &rest[start + begin.len()..];
        let Some(stop) = after.find(&end) else { break };
        let body: String = after[..stop].split_whitespace().collect();
        if let Some(bytes) = base64_decode(&body) {
            blocks.push(bytes);
        }
        rest = &after[stop + end.len()..];
    }
    blocks
}

fn pem_certificates(pem: &str) -> CertResult<Vec<CertificateDer<'static>>> {
    Ok(pem_blocks(pem, "CERTIFICATE")
        .into_iter()
        .map(CertificateDer::from)
        .collect())
}

fn pem_private_key(pem: &str) -> CertResult<PrivateKeyDer<'static>> {
    for label in ["PRIVATE KEY", "RSA PRIVATE KEY", "EC PRIVATE KEY"] {
        if let Some(block) = pem_blocks(pem, label).into_iter().next() {
            return PrivateKeyDer::try_from(block)
                .map_err(|error| CertError::Invalid(format!("unreadable private key: {error}")));
        }
    }
    Err(CertError::Invalid(
        "no private key in the stored PEM".into(),
    ))
}

fn base64_decode(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        lookup[*byte as usize] = index as u8;
    }

    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if byte == b'=' {
            break;
        }
        let value = lookup[byte as usize];
        if value == 255 {
            return None;
        }
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

// ---------- issuing ---------------------------------------------------

struct LocalCa {
    /// The certificate exactly as stored — the bytes an operator
    /// fingerprints and trusts.
    ///
    /// Kept separately from `issuer` because reconstructing a
    /// certificate through rcgen **re-signs** it, and ECDSA signatures
    /// are randomized: the same authority serialized twice is
    /// byte-different. Handing out the regenerated form would change
    /// the fingerprint on every boot, and an operator who is asked to
    /// re-trust the CA each restart will stop reading the prompt.
    pem: String,
    /// Only for signing leaves. Never serialized out.
    issuer: rcgen::Certificate,
    key: rcgen::KeyPair,
}

/// Load the node's certificate authority, generating it on first call.
async fn local_ca(database: &SqliteDatabase) -> CertResult<LocalCa> {
    let existing: Option<(String, String)> = database
        .read(|connection| {
            connection
                .query_row(
                    "SELECT \"cert_pem\", \"key_pem\" FROM local_ca WHERE \"id\" = 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
        })
        .await?;

    if let Some((cert_pem, key_pem)) = existing {
        let key = rcgen::KeyPair::from_pem(&key_pem)?;
        // Rebuilt from its own parameters rather than stored as a
        // `Certificate`: rcgen needs the params to sign with, and
        // re-deriving them is what keeps the on-disk form plain PEM
        // that an operator can inspect.
        let issuer = rcgen::CertificateParams::from_ca_cert_pem(&cert_pem)?.self_signed(&key)?;
        return Ok(LocalCa {
            pem: cert_pem,
            issuer,
            key,
        });
    }

    let key = rcgen::KeyPair::generate()?;
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new())?;
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0));
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "wabot-deploy local CA");
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc() + Duration::days(CA_YEARS * 365);
    let issuer = params.self_signed(&key)?;

    let (cert_pem, key_pem) = (issuer.pem(), key.serialize_pem());
    let stored_pem = cert_pem.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT OR IGNORE INTO local_ca (\"id\", \"cert_pem\", \"key_pem\", \"created_at\") \
                 VALUES (1, ?1, ?2, ?3)",
                (stored_pem, key_pem, now_ms()),
            )?;
            Ok(())
        })
        .await?;

    tracing::info!("generated a local certificate authority");
    Ok(LocalCa {
        pem: cert_pem,
        issuer,
        key,
    })
}

/// Where `install` exports the CA for a trust store.
pub fn ca_certificate_path(config: &crate::config::Config) -> std::path::PathBuf {
    config.certificates_dir().join("local-ca.crt")
}

/// The CA certificate in PEM, for an operator to trust.
pub async fn ca_certificate_pem(database: &SqliteDatabase) -> CertResult<String> {
    Ok(local_ca(database).await?.pem)
}

/// Ensure a locally-issued certificate exists covering `names`.
///
/// Returns the stored certificate. Reissues when the existing one does
/// not cover every name asked for, or expires within a month.
pub async fn ensure_self_signed(
    database: &SqliteDatabase,
    primary: &str,
    names: &[String],
) -> CertResult<StoredCert> {
    let wanted = canonical(names);
    if wanted.is_empty() {
        return Err(CertError::Invalid(
            "a certificate needs at least one name".into(),
        ));
    }

    if let Some(existing) = load(database, primary).await? {
        // Reissue when it is close to expiry *or* no longer covers
        // what is asked for — configuring a domain is the second case,
        // and a node that kept serving the old certificate would be
        // presenting the wrong name with no sign of why.
        let fresh = existing.not_after > now_ms() + RENEW_WITHIN_DAYS * 86_400_000;
        if fresh && existing.names == wanted {
            return Ok(existing);
        }
    }

    let ca = local_ca(database).await?;
    let key = rcgen::KeyPair::generate()?;
    let mut params = rcgen::CertificateParams::new(wanted.clone())?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, primary);
    params.not_before = OffsetDateTime::now_utc();
    params.not_after = OffsetDateTime::now_utc() + Duration::days(SELF_SIGNED_DAYS);
    let leaf = params.signed_by(&key, &ca.issuer, &ca.key)?;

    // The CA is appended so a client that trusts only the root can
    // still build the chain — without it, trusting the CA is not
    // enough and the warning stays.
    let stored = StoredCert {
        domain: primary.to_string(),
        names: wanted,
        cert_pem: format!("{}{}", leaf.pem(), ca.pem),
        key_pem: key.serialize_pem(),
        issuer: "self-signed".to_string(),
        not_after: now_ms() + SELF_SIGNED_DAYS * 86_400_000,
    };
    save(database, &stored).await?;
    tracing::info!(
        domain = %primary,
        names = stored.names.join(", "),
        "issued a self-signed certificate"
    );
    Ok(stored)
}

pub async fn load(database: &SqliteDatabase, domain: &str) -> CertResult<Option<StoredCert>> {
    let domain = domain.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"domain\", \"names\", \"cert_pem\", \"key_pem\", \"issuer\", \"not_after\" \
                     FROM certificate WHERE \"domain\" = ?1",
                    [domain],
                    |row| {
                        Ok(StoredCert {
                            domain: row.get(0)?,
                            names: split_names(&row.get::<_, String>(1)?),
                            cert_pem: row.get(2)?,
                            key_pem: row.get(3)?,
                            issuer: row.get(4)?,
                            not_after: row.get(5)?,
                        })
                    },
                )
                .optional()
        })
        .await?)
}

pub async fn load_all(database: &SqliteDatabase) -> CertResult<Vec<StoredCert>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"domain\", \"names\", \"cert_pem\", \"key_pem\", \"issuer\", \"not_after\" \
                     FROM certificate ORDER BY \"domain\"",
                )?
                .query_map([], |row| {
                    Ok(StoredCert {
                        domain: row.get(0)?,
                        names: split_names(&row.get::<_, String>(1)?),
                        cert_pem: row.get(2)?,
                        key_pem: row.get(3)?,
                        issuer: row.get(4)?,
                        not_after: row.get(5)?,
                    })
                })?
                .collect()
        })
        .await?)
}

/// Store a certificate, whoever issued it.
pub async fn save(database: &SqliteDatabase, stored: &StoredCert) -> CertResult<()> {
    let stored = stored.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO certificate \
                   (\"domain\", \"names\", \"cert_pem\", \"key_pem\", \"issuer\", \"issued_at\", \"not_after\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
                 ON CONFLICT (\"domain\") DO UPDATE SET \
                   \"names\" = excluded.\"names\", \
                   \"cert_pem\" = excluded.\"cert_pem\", \
                   \"key_pem\" = excluded.\"key_pem\", \
                   \"issuer\" = excluded.\"issuer\", \
                   \"issued_at\" = excluded.\"issued_at\", \
                   \"not_after\" = excluded.\"not_after\", \
                   \"last_error\" = NULL",
                (
                    stored.domain,
                    stored.names.join(","),
                    stored.cert_pem,
                    stored.key_pem,
                    stored.issuer,
                    now_ms(),
                    stored.not_after,
                ),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// When a certificate expires, in epoch millis.
///
/// Read from the leaf rather than assumed, because an ACME certificate
/// is issued for whatever window the authority chose — Let's Encrypt
/// says 90 days today and has said it will shorten that. A renewal
/// schedule built on a guess is one that stops renewing in time.
///
/// `None` when the certificate cannot be parsed, which the caller
/// treats as "renew soon" rather than "never expires".
pub fn not_after(cert_pem: &str) -> Option<i64> {
    let der = pem_blocks(cert_pem, "CERTIFICATE").into_iter().next()?;
    let (_, parsed) = x509_parser::parse_x509_certificate(&der).ok()?;
    Some(parsed.validity().not_after.timestamp() * 1000)
}

fn split_names(joined: &str) -> Vec<String> {
    joined
        .split(',')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database() -> SqliteDatabase {
        crate::db::open_in_memory().await.expect("open")
    }

    #[tokio::test]
    async fn a_certificate_is_issued_and_reused() {
        let database = database().await;
        let names = vec!["localhost".to_string()];

        let first = ensure_self_signed(&database, "localhost", &names)
            .await
            .expect("issue");
        assert_eq!(first.issuer, "self-signed");
        assert!(first.cert_pem.contains("BEGIN CERTIFICATE"));

        let second = ensure_self_signed(&database, "localhost", &names)
            .await
            .expect("reuse");
        assert_eq!(
            first.cert_pem, second.cert_pem,
            "a valid certificate is reused rather than reissued on every boot"
        );
    }

    /// The chain has to carry the CA, or trusting the CA is not enough
    /// and the client still complains.
    #[tokio::test]
    async fn the_chain_includes_the_authority() {
        let database = database().await;
        let stored = ensure_self_signed(&database, "localhost", &["localhost".into()])
            .await
            .expect("issue");
        assert_eq!(
            stored.cert_pem.matches("BEGIN CERTIFICATE").count(),
            2,
            "leaf plus the CA that signed it"
        );
    }

    #[tokio::test]
    async fn the_authority_is_generated_once() {
        let database = database().await;
        let first = ca_certificate_pem(&database).await.expect("ca");
        let second = ca_certificate_pem(&database).await.expect("ca");
        assert_eq!(first, second, "a second call must not mint a new authority");
    }

    #[tokio::test]
    async fn a_new_name_forces_a_reissue() {
        let database = database().await;
        let first = ensure_self_signed(&database, "localhost", &["localhost".into()])
            .await
            .expect("issue");
        let second = ensure_self_signed(
            &database,
            "localhost",
            &["localhost".into(), "node.example.com".into()],
        )
        .await
        .expect("reissue");
        assert_ne!(
            first.cert_pem, second.cert_pem,
            "configuring a domain has to produce a certificate that covers it"
        );
        assert!(second.cert_pem.contains("BEGIN CERTIFICATE"));
    }

    #[tokio::test]
    async fn the_resolver_serves_by_name_and_falls_back() {
        let database = database().await;
        ensure_self_signed(&database, "localhost", &["localhost".into()])
            .await
            .expect("issue");

        let resolver = CertResolver::new();
        resolver
            .replace(&load_all(&database).await.expect("load"))
            .expect("replace");

        assert_eq!(resolver.names(), vec!["localhost".to_string()]);
        assert!(
            resolver.fallback.load().is_some(),
            "a handshake with no SNI still needs an answer"
        );
    }

    #[test]
    fn pem_parsing_finds_every_block() {
        let pem = "-----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----\n\
                   -----BEGIN CERTIFICATE-----\nREVG\n-----END CERTIFICATE-----\n";
        let blocks = pem_blocks(pem, "CERTIFICATE");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0], b"ABC");
        assert_eq!(blocks[1], b"DEF");
    }

    #[test]
    fn base64_round_trips_the_shapes_pem_produces() {
        assert_eq!(base64_decode("QUJD").unwrap(), b"ABC");
        assert_eq!(base64_decode("QUJDRA==").unwrap(), b"ABCD");
        assert_eq!(base64_decode("QUJDREU=").unwrap(), b"ABCDE");
        assert!(base64_decode("not base64!").is_none());
    }
}
