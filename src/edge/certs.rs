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
    /// Where this came from, as opposed to who signed it.
    ///
    /// `issuer` was carrying both, and the renewal loop read it as a
    /// decision — "is this from the authority I am configured for" —
    /// which silently replaced anything it did not recognise. See
    /// migration `0012`.
    pub source: Source,
}

/// Where a certificate came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    SelfSigned,
    Acme,
    /// Read off disk, kept fresh by something outside this node.
    File,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SelfSigned => "self_signed",
            Self::Acme => "acme",
            Self::File => "file",
        }
    }

    /// Unknown reads as self-signed: it is the one answer that is safe
    /// to act on, because reissuing a self-signed certificate cannot
    /// throw away something nobody here could make again.
    pub fn parse(text: &str) -> Self {
        match text {
            "acme" => Self::Acme,
            "file" => Self::File,
            _ => Self::SelfSigned,
        }
    }
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
        source: Source::SelfSigned,
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
                    "SELECT \"domain\", \"names\", \"cert_pem\", \"key_pem\", \"issuer\", \"not_after\", \"source\" \
                     FROM certificate WHERE \"domain\" = ?1",
                    [domain],
                    |row| {
                        Ok(StoredCert {
                            domain: row.get(0)?,
                            names: split_names(&row.get::<_, String>(1)?),
                            cert_pem: row.get(2)?,
                            key_pem: row.get(3)?,
                            issuer: row.get(4)?,
                            source: Source::parse(&row.get::<_, String>(6)?),
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
                    "SELECT \"domain\", \"names\", \"cert_pem\", \"key_pem\", \"issuer\", \"not_after\", \"source\" \
                     FROM certificate ORDER BY \"domain\"",
                )?
                .query_map([], |row| {
                    Ok(StoredCert {
                        domain: row.get(0)?,
                        names: split_names(&row.get::<_, String>(1)?),
                        cert_pem: row.get(2)?,
                        key_pem: row.get(3)?,
                        issuer: row.get(4)?,
                        source: Source::parse(&row.get::<_, String>(6)?),
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
                   (\"domain\", \"names\", \"cert_pem\", \"key_pem\", \"issuer\", \"issued_at\", \"not_after\", \"source\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT (\"domain\") DO UPDATE SET \
                   \"names\" = excluded.\"names\", \
                   \"cert_pem\" = excluded.\"cert_pem\", \
                   \"key_pem\" = excluded.\"key_pem\", \
                   \"issuer\" = excluded.\"issuer\", \
                   \"issued_at\" = excluded.\"issued_at\", \
                   \"not_after\" = excluded.\"not_after\", \
                   \"source\" = excluded.\"source\", \
                   \"last_error\" = NULL",
                (
                    stored.domain,
                    stored.names.join(","),
                    stored.cert_pem,
                    stored.key_pem,
                    stored.issuer,
                    now_ms(),
                    stored.not_after,
                    stored.source.as_str(),
                ),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Forget every stored certificate this node no longer has a name for.
///
/// Renaming a node, clearing its domain, dropping an edge, or a database
/// learning it belongs to somebody else all leave a certificate keyed
/// under the old name. Nothing removed them, so each one was reissued
/// for ever and listed by `doctor` — a page of names the node does not
/// answer for, which is the sort of report that teaches people to skip
/// reading the report.
///
/// Two rules keep this from being the mistake it could be:
///
/// - **`Source::File` is never touched.** A self-signed one this node
///   can make again and an ACME one it can order again; a file somebody
///   put here it cannot recreate, and a convergent pass must not be able
///   to destroy something unrecoverable.
/// - **The caller must know the full answer.** `wanted` is every name
///   this node still wants a certificate under, from every source of
///   them — and a caller that could not read one of those sources must
///   pass nothing rather than a short list, because a short list here
///   deletes working certificates. Hence [`prune`] returning early on an
///   empty `wanted`: there is no node that legitimately wants none, so
///   an empty list is a failure upstream rather than an instruction.
pub async fn prune(database: &SqliteDatabase, wanted: &[String]) -> CertResult<Vec<String>> {
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let stale: Vec<String> = load_all(database)
        .await?
        .into_iter()
        .filter(|stored| stored.source != Source::File)
        .filter(|stored| !wanted.iter().any(|name| name == &stored.domain))
        .map(|stored| stored.domain)
        .collect();

    for domain in &stale {
        let domain = domain.clone();
        database
            .write(move |connection| {
                connection.execute("DELETE FROM certificate WHERE \"domain\" = ?1", [domain])?;
                Ok(())
            })
            .await?;
    }
    Ok(stale)
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

/// Every DNS name a certificate says it is for.
///
/// Read out of the leaf, unlike `StoredCert::names` for the ones this
/// node issues — there we know what we asked for. A certificate that
/// arrived from somewhere else has to be asked.
pub fn names_in(cert_pem: &str) -> Vec<String> {
    let Some(der) = pem_blocks(cert_pem, "CERTIFICATE").into_iter().next() else {
        return Vec::new();
    };
    let Ok((_, parsed)) = x509_parser::parse_x509_certificate(&der) else {
        return Vec::new();
    };
    let Ok(Some(alternative)) = parsed.subject_alternative_name() else {
        return Vec::new();
    };
    let mut names: Vec<String> = alternative
        .value
        .general_names
        .iter()
        .filter_map(|name| match name {
            x509_parser::extensions::GeneralName::DNSName(dns) => Some(dns.to_string()),
            _ => None,
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Does one of `names` answer for `wanted`?
///
/// Exact, case-insensitively. No wildcards: a name here is a name the
/// node serves, and every one of them is written down — the node's
/// domain, or a port's hostname. Matching `*.example.com` would mean
/// the resolver has to as well, and it does not: it looks a name up in
/// a map. Accepting one here and failing to serve it is worse than
/// refusing it, because the console would report the node configured
/// and no browser would open it.
pub fn covers(names: &[String], wanted: &str) -> bool {
    names.iter().any(|name| name.eq_ignore_ascii_case(wanted))
}

/// Read a certificate and its key off disk, refusing anything that
/// would take the node off the air.
///
/// Every check here is one that otherwise surfaces as a failed
/// handshake — and the console is served over the same listener, so a
/// certificate that breaks TLS breaks the only page anybody could fix
/// it from. This is the one place in the node where a private key
/// arrives from outside, and it is the last moment it can be refused.
pub fn from_files(name: &str, cert_path: &str, key_path: &str) -> CertResult<StoredCert> {
    let read = |path: &str| {
        std::fs::read_to_string(path)
            .map_err(|error| CertError::Invalid(format!("could not read {path}: {error}")))
    };
    let cert_pem = read(cert_path)?;
    let key_pem = read(key_path)?;

    let names = names_in(&cert_pem);
    if names.is_empty() {
        return Err(CertError::Invalid(format!(
            "{cert_path} has no DNS names in it — a certificate with no subject \
             alternative name is one no browser will accept"
        )));
    }
    if !covers(&names, name) {
        // A wildcard is the near miss worth naming. "is for
        // *.example.com — not api.example.com" reads like a typo, and
        // somebody would spend a while checking a file that is exactly
        // what they thought it was.
        let wildcard = names.iter().any(|found| found.starts_with("*."));
        return Err(CertError::Invalid(if wildcard {
            format!(
                "{cert_path} is a wildcard certificate ({}), and this node serves \
                 names one at a time — give it a certificate for {name} itself",
                names.join(", ")
            )
        } else {
            format!("{cert_path} is for {} — not {name}", names.join(", "))
        }));
    }

    let Some(not_after) = not_after(&cert_pem) else {
        return Err(CertError::Invalid(format!(
            "could not read an expiry from {cert_path}"
        )));
    };
    if not_after <= now_ms() {
        return Err(CertError::Invalid(format!(
            "{cert_path} expired already — installing it would take this node off \
             the air rather than keep it on"
        )));
    }

    let stored = StoredCert {
        domain: name.to_string(),
        // Who signed it stays a fact read off the certificate; `source`
        // records how the node came by it. Writing "file" into both
        // would be the same conflation migration `0012` undid.
        issuer: issuer_of(&cert_pem).unwrap_or_else(|| "unknown".into()),
        names,
        cert_pem,
        key_pem,
        not_after,
        source: Source::File,
    };

    // Last, because it is the expensive one and because the errors
    // above are the ones somebody can act on. A key that does not
    // belong to the certificate is the failure that would otherwise
    // reach a browser as a handshake alert with no explanation.
    certified_key(&stored)?.keys_match().map_err(|error| {
        CertError::Invalid(format!("{key_path} does not open {cert_path}: {error}"))
    })?;

    Ok(stored)
}

/// Who signed a certificate, as its issuer's common name.
///
/// For anything this node did not issue: an ACME certificate records
/// the directory URL it came from, and a self-signed one says so, but
/// a certificate found on disk has to be asked.
pub fn issuer_of(cert_pem: &str) -> Option<String> {
    let der = pem_blocks(cert_pem, "CERTIFICATE").into_iter().next()?;
    let (_, parsed) = x509_parser::parse_x509_certificate(&der).ok()?;
    let common_name = parsed
        .issuer()
        .iter_common_name()
        .next()
        .and_then(|name| name.as_str().ok())
        .map(str::to_string);
    // Bound to a local before returning: the parse borrows `der`, and
    // the name borrows the parse.
    common_name
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

    /// A certificate and key on disk, as something outside this node
    /// would have left them.
    fn write_pair(dir: &std::path::Path, names: &[&str]) -> (String, String) {
        let key = rcgen::KeyPair::generate().expect("key");
        let params = rcgen::CertificateParams::new(
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
        )
        .expect("params");
        let certificate = params.self_signed(&key).expect("sign");

        let cert_path = dir.join("node.crt");
        let key_path = dir.join("node.key");
        std::fs::write(&cert_path, certificate.pem()).expect("write cert");
        std::fs::write(&key_path, key.serialize_pem()).expect("write key");
        (
            cert_path.to_string_lossy().into_owned(),
            key_path.to_string_lossy().into_owned(),
        )
    }

    #[test]
    fn a_pair_on_disk_is_read_and_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (cert_path, key_path) = write_pair(dir.path(), &["node.example.com"]);

        let stored = from_files("node.example.com", &cert_path, &key_path).expect("accepted");
        assert_eq!(stored.source, Source::File);
        assert_eq!(stored.names, vec!["node.example.com".to_string()]);
        assert!(stored.not_after > now_ms(), "and it is still valid");
    }

    /// The failure this refuses is the expensive one: a mismatched pair
    /// installs cleanly and then breaks every handshake, including the
    /// one serving the page somebody would fix it from.
    #[test]
    fn a_key_that_does_not_open_the_certificate_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (cert_path, _) = write_pair(dir.path(), &["node.example.com"]);
        let other = tempfile::tempdir().expect("tempdir");
        let (_, key_path) = write_pair(other.path(), &["node.example.com"]);

        let error = from_files("node.example.com", &cert_path, &key_path).expect_err("refused");
        assert!(error.to_string().contains("does not open"), "{error}");
    }

    #[test]
    fn a_certificate_for_another_name_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (cert_path, key_path) = write_pair(dir.path(), &["other.example.com"]);

        let error = from_files("node.example.com", &cert_path, &key_path).expect_err("refused");
        assert!(
            error.to_string().contains("not node.example.com"),
            "{error}"
        );
    }

    #[test]
    fn a_missing_file_says_which_one() {
        let error = from_files("node.example.com", "/nowhere/node.crt", "/nowhere/node.key")
            .expect_err("refused");
        assert!(error.to_string().contains("/nowhere/node.crt"), "{error}");
    }

    /// Names match exactly. The resolver looks a name up in a map, so a
    /// wildcard accepted here would be a certificate the node stored
    /// and then never served.
    #[test]
    fn a_name_matches_itself_and_nothing_else() {
        let names = vec!["api.example.com".to_string()];
        assert!(covers(&names, "api.example.com"));
        assert!(covers(&names, "API.example.com"), "case does not matter");
        assert!(!covers(&names, "example.com"));
        assert!(!covers(&["*.example.com".to_string()], "api.example.com"));
    }

    /// A wildcard is the near miss worth explaining: "is for
    /// *.example.com — not api.example.com" reads like a typo, and
    /// somebody would go looking for a mistake in a file that is
    /// exactly what they thought it was.
    #[test]
    fn a_wildcard_is_refused_by_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (cert_path, key_path) = write_pair(dir.path(), &["*.example.com"]);

        let error = from_files("api.example.com", &cert_path, &key_path).expect_err("refused");
        assert!(error.to_string().contains("wildcard"), "{error}");
        assert!(
            error.to_string().contains("api.example.com itself"),
            "and says what would work: {error}"
        );
    }

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

    /// The store used to keep a certificate for ever once it had one.
    /// Renaming a node, clearing a domain, dropping an edge or a
    /// database learning it belongs to somebody else all left a row
    /// behind, reissued twice a day for a name the node does not answer
    /// for — and `doctor` listed every one of them.
    #[tokio::test]
    async fn a_certificate_for_a_name_this_node_stopped_serving_is_forgotten() {
        let database = crate::db::open_in_memory().await.expect("open");
        for name in ["kept.example", "gone.example"] {
            ensure_self_signed(&database, name, &[name.to_string()])
                .await
                .expect("issued");
        }

        let gone = prune(&database, &["kept.example".to_string()])
            .await
            .expect("pruned");

        assert_eq!(gone, vec!["gone.example".to_string()]);
        let left: Vec<String> = load_all(&database)
            .await
            .expect("load")
            .into_iter()
            .map(|stored| stored.domain)
            .collect();
        assert_eq!(left, vec!["kept.example".to_string()]);
    }

    /// A certificate somebody put on this node is the one thing here it
    /// cannot make again. A self-signed one it can reissue and an ACME
    /// one it can re-order; a file it can only lose. So a convergent
    /// pass may not be able to destroy it, however unwanted the name.
    #[tokio::test]
    async fn a_certificate_from_a_file_is_never_pruned() {
        let database = crate::db::open_in_memory().await.expect("open");
        let dir = tempfile::tempdir().expect("tempdir");
        let (cert_path, key_path) = write_pair(dir.path(), &["theirs.example"]);
        let found = from_files("theirs.example", &cert_path, &key_path).expect("read");
        save(&database, &found).await.expect("save");

        let gone = prune(&database, &["something.else".to_string()])
            .await
            .expect("pruned");

        assert!(gone.is_empty(), "{gone:?}");
        assert!(load(&database, "theirs.example")
            .await
            .expect("load")
            .is_some());
    }

    /// The dangerous case, and the reason the empty list is a special
    /// one. Three unrelated sources feed the wanted set; a caller that
    /// failed to read one of them has a *short* list, and a short list
    /// deletes certificates that were working. There is no node that
    /// legitimately wants none, so an empty list is a bug upstream
    /// rather than an instruction to empty the store.
    #[tokio::test]
    async fn an_empty_wanted_list_deletes_nothing() {
        let database = crate::db::open_in_memory().await.expect("open");
        ensure_self_signed(&database, "live.example", &["live.example".to_string()])
            .await
            .expect("issued");

        assert!(prune(&database, &[]).await.expect("pruned").is_empty());
        assert!(load(&database, "live.example")
            .await
            .expect("load")
            .is_some());
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
