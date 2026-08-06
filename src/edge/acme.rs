//! Getting a real certificate, and keeping it.
//!
//! ## HTTP-01, and why the challenge lives in the database
//!
//! The certificate authority validates by fetching
//! `http://<domain>/.well-known/acme-challenge/<token>` and comparing
//! what comes back. The node already owns port 80 for the HTTPS
//! redirect, so it can answer — but an order can be in flight when the
//! process restarts, and an in-memory answer would 404 after that and
//! fail the order for no reason a log would explain.
//!
//! ## Nothing here runs inside a handshake
//!
//! `ResolvesServerCert::resolve` is synchronous and cannot await an
//! issuance, so the ordering is: obtain, store, then swap the resolver.
//! A hostname with no certificate keeps getting the local authority's
//! one — a warning the operator can click through — rather than a
//! failed handshake they cannot.
//!
//! ## The account outlives the process
//!
//! Registering a new account on every start would be both rude and a
//! good way to hit a rate limit. Credentials are stored per directory
//! URL, so staging and production accounts coexist and switching
//! between them is a config change rather than a loss.

use std::sync::Arc;

use instant_acme::{
    Account, AccountCredentials, AuthorizationStatus, ChallengeType, Identifier, NewAccount,
    NewOrder, OrderStatus, RetryPolicy,
};
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use super::certs::{self, StoredCert};
use crate::config::Config;

/// Renew once the certificate is inside this window of expiring.
///
/// Let's Encrypt issues for 90 days and starts mailing at 20 left; 30
/// gives the renewal loop several chances to succeed — and a node
/// whose renewal has been failing for a week still has a fortnight for
/// somebody to notice.
const RENEW_WITHIN_DAYS: i64 = 30;

/// How long a stored challenge is worth answering.
///
/// An order the CA never validates leaves its challenge behind; this
/// is what stops the table growing forever.
const CHALLENGE_TTL_SECONDS: i64 = 3600;

pub const CHALLENGE_PREFIX: &str = "/.well-known/acme-challenge/";

#[derive(Debug, thiserror::Error)]
pub enum AcmeError {
    #[error("storage: {0}")]
    Storage(#[from] wabot::sqlite::SqliteError),
    #[error("certificates: {0}")]
    Certificates(#[from] certs::CertError),
    #[error("acme: {0}")]
    Protocol(#[from] instant_acme::Error),
    #[error("serializing account credentials: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("{0}")]
    Refused(String),
}

pub type AcmeResult<T> = Result<T, AcmeError>;

// ---------- the challenge responder -----------------------------------

/// The answer to one HTTP-01 challenge, or `None`.
pub async fn challenge_response(
    database: &SqliteDatabase,
    token: &str,
) -> AcmeResult<Option<String>> {
    let token = token.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"response\" FROM acme_challenge WHERE \"token\" = ?1",
                    [token],
                    |row| row.get::<_, String>(0),
                )
                .optional()
        })
        .await?)
}

async fn store_challenge(
    database: &SqliteDatabase,
    token: &str,
    response: &str,
    domain: &str,
) -> AcmeResult<()> {
    let (token, response, domain) = (token.to_string(), response.to_string(), domain.to_string());
    let expires = now_ms() + CHALLENGE_TTL_SECONDS * 1000;
    database
        .write(move |connection| {
            // Expired rows are swept here rather than on a timer: this
            // is the only place that adds one, so it is the only place
            // that needs to care.
            connection.execute(
                "DELETE FROM acme_challenge WHERE \"expires_at\" < ?1",
                [now_ms()],
            )?;
            connection.execute(
                "INSERT INTO acme_challenge (\"token\", \"response\", \"domain\", \"expires_at\") \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (\"token\") DO UPDATE SET \
                   \"response\" = excluded.\"response\", \
                   \"expires_at\" = excluded.\"expires_at\"",
                (token, response, domain, expires),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

async fn clear_challenges(database: &SqliteDatabase, domain: &str) -> AcmeResult<()> {
    let domain = domain.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM acme_challenge WHERE \"domain\" = ?1", [domain])?;
            Ok(())
        })
        .await?;
    Ok(())
}

// ---------- the account -----------------------------------------------

async fn load_or_create_account(database: &SqliteDatabase, config: &Config) -> AcmeResult<Account> {
    let directory = config.acme.directory_url().to_string();

    let stored: Option<String> = database
        .read({
            let directory = directory.clone();
            move |connection| {
                connection
                    .query_row(
                        "SELECT \"credentials\" FROM acme_account WHERE \"directory_url\" = ?1",
                        [directory],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
            }
        })
        .await?;

    if let Some(json) = stored {
        let credentials: AccountCredentials = serde_json::from_str(&json)?;
        tracing::debug!(%directory, "reusing the stored ACME account");
        return Ok(Account::builder()?.from_credentials(credentials).await?);
    }

    let contact: Vec<String> = config
        .acme
        .email
        .iter()
        .map(|email| format!("mailto:{email}"))
        .collect();
    let contact: Vec<&str> = contact.iter().map(String::as_str).collect();

    let (account, credentials) = Account::builder()?
        .create(
            &NewAccount {
                contact: &contact,
                // Registering *is* the agreement; the CA has no other
                // way to be told, and refusing to set it would just
                // mean no account.
                terms_of_service_agreed: true,
                only_return_existing: false,
            },
            directory.clone(),
            None,
        )
        .await?;

    let json = serde_json::to_string(&credentials)?;
    database
        .write({
            let directory = directory.clone();
            let email = config.acme.email.clone();
            move |connection| {
                connection.execute(
                    "INSERT INTO acme_account \
                       (\"directory_url\", \"email\", \"credentials\", \"created_at\") \
                     VALUES (?1, ?2, ?3, ?4) \
                     ON CONFLICT (\"directory_url\") DO UPDATE SET \
                       \"credentials\" = excluded.\"credentials\", \
                       \"email\" = excluded.\"email\"",
                    (directory, email, json, now_ms()),
                )?;
                Ok(())
            }
        })
        .await?;

    tracing::info!(%directory, "registered an ACME account");
    Ok(account)
}

// ---------- issuing ----------------------------------------------------

/// Obtain a certificate for `domain`, storing it on success.
///
/// The whole order, start to finish: authorize, answer the challenge,
/// finalize, poll. Long-running by nature — the CA takes seconds to
/// validate — which is why the caller runs it off the request path.
pub async fn obtain(
    database: &SqliteDatabase,
    config: &Config,
    domain: &str,
) -> AcmeResult<StoredCert> {
    let account = load_or_create_account(database, config).await?;

    let identifiers = [Identifier::Dns(domain.to_string())];
    let mut order = account.new_order(&NewOrder::new(&identifiers)).await?;

    // Answer every pending authorization before telling the CA to
    // look: `set_ready` is what starts validation, and a token the
    // node cannot yet serve fails the order immediately.
    let mut authorizations = order.authorizations();
    while let Some(result) = authorizations.next().await {
        let mut authorization = result?;
        match authorization.status {
            AuthorizationStatus::Pending => {}
            // Already valid from an earlier order — the CA caches
            // these, which is what makes a retry cheap.
            AuthorizationStatus::Valid => continue,
            other => {
                return Err(AcmeError::Refused(format!(
                    "the authority will not validate {domain}: {other:?}"
                )))
            }
        }

        let mut challenge = authorization
            .challenge(ChallengeType::Http01)
            .ok_or_else(|| {
                AcmeError::Refused("the authority offered no http-01 challenge".into())
            })?;

        let token = challenge.token.clone();
        let response = challenge.key_authorization().as_str().to_string();
        store_challenge(database, &token, &response, domain).await?;
        tracing::info!(%domain, %token, "answering an http-01 challenge");

        challenge.set_ready().await?;
    }

    let status = order.poll_ready(&RetryPolicy::default()).await?;
    if status != OrderStatus::Ready {
        clear_challenges(database, domain).await?;
        return Err(AcmeError::Refused(format!(
            "the order for {domain} ended {status:?} — \
             check that http://{domain}{CHALLENGE_PREFIX}… reaches this node"
        )));
    }

    let key_pem = order.finalize().await?;
    let chain_pem = order.poll_certificate(&RetryPolicy::default()).await?;
    clear_challenges(database, domain).await?;

    let stored = StoredCert {
        domain: domain.to_string(),
        names: vec![domain.to_string()],
        not_after: certs::not_after(&chain_pem)
            // Unparseable means renew at the next pass rather than
            // trust a number nobody read.
            .unwrap_or_else(now_ms),
        cert_pem: chain_pem,
        key_pem,
        issuer: if config.acme.is_staging() {
            "acme-staging"
        } else {
            "acme"
        }
        .to_string(),
    };
    certs::save(database, &stored).await?;

    tracing::info!(%domain, "obtained a certificate");
    Ok(stored)
}

/// Obtain a certificate if one is needed, and swap it in.
///
/// Returns whether anything changed. Idempotent and cheap when the
/// certificate is current, so it is safe to call on every start and on
/// every pass of the renewal loop.
pub async fn ensure(
    database: &SqliteDatabase,
    config: &Config,
    resolver: &certs::CertResolver,
) -> AcmeResult<bool> {
    if config.acme.disabled {
        return Ok(false);
    }
    let Some(domain) = config.node.domain.clone() else {
        // No domain, nothing a public CA could validate. The local
        // authority's certificate stands.
        return Ok(false);
    };

    if let Some(existing) = certs::load(database, &domain).await? {
        let acme_issued = existing.issuer.starts_with("acme");
        let fresh = existing.not_after > now_ms() + RENEW_WITHIN_DAYS * 86_400_000;
        if acme_issued && fresh {
            return Ok(false);
        }
    }

    obtain(database, config, &domain).await?;
    // Reloaded from storage rather than pushed in: the resolver then
    // serves exactly what a restart would, so there is no state that
    // exists only in this process.
    resolver.replace(&certs::load_all(database).await?)?;
    Ok(true)
}

/// The background loop: try until it works, then keep it renewed.
///
/// Failure is expected here rather than exceptional — DNS propagates
/// slowly, a firewall gets opened late — so this retries with backoff
/// instead of giving up, and the node keeps serving throughout on the
/// local authority's certificate.
pub async fn renewal_loop(
    database: Arc<SqliteDatabase>,
    config: Config,
    resolver: Arc<certs::CertResolver>,
    cancel: wabot::lifecycle::Cancel,
) -> Result<(), std::convert::Infallible> {
    if config.acme.disabled || config.node.domain.is_none() {
        tracing::info!("ACME is not configured; serving the local authority's certificate");
        // Not a return: a service that ends takes the process with it,
        // and "no domain configured" is a fine way to run.
        cancel.cancelled().await;
        return Ok(());
    }

    // Start at a minute and back off to six hours. The first attempts
    // are the ones most likely to be waiting on DNS the operator is
    // still setting up.
    let mut delay = std::time::Duration::from_secs(60);
    const MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);
    // Once a certificate is in hand, checking twice a day is plenty
    // against a 30-day window.
    const SETTLED_DELAY: std::time::Duration = std::time::Duration::from_secs(12 * 3600);

    loop {
        match ensure(&database, &config, &resolver).await {
            Ok(changed) => {
                if changed {
                    tracing::info!(
                        names = resolver.names().join(", "),
                        "certificate installed without a restart"
                    );
                }
                delay = SETTLED_DELAY;
            }
            Err(error) => {
                tracing::warn!(%error, retry_in = ?delay, "could not obtain a certificate");
                if let Err(error) = record_failure(&database, &config, &error).await {
                    tracing::debug!(%error, "could not record the failure");
                }
                delay = (delay * 2).min(MAX_DELAY);
            }
        }

        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(delay) => {}
        }
    }
}

/// Leave the reason on the certificate row, so `doctor` can show it
/// without an operator reading the journal.
async fn record_failure(
    database: &SqliteDatabase,
    config: &Config,
    error: &AcmeError,
) -> AcmeResult<()> {
    let Some(domain) = config.node.domain.clone() else {
        return Ok(());
    };
    let message = error.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE certificate SET \"last_error\" = ?2 WHERE \"domain\" = ?1",
                (domain, message),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
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
    async fn a_challenge_is_answered_and_then_forgotten() {
        let database = database().await;

        assert_eq!(
            challenge_response(&database, "tok").await.expect("read"),
            None
        );

        store_challenge(&database, "tok", "tok.keyauth", "node.example.com")
            .await
            .expect("store");
        assert_eq!(
            challenge_response(&database, "tok").await.expect("read"),
            Some("tok.keyauth".to_string())
        );

        clear_challenges(&database, "node.example.com")
            .await
            .expect("clear");
        assert_eq!(
            challenge_response(&database, "tok").await.expect("read"),
            None,
            "a finished order leaves nothing behind"
        );
    }

    /// The answer has to survive a restart: an order can be in flight
    /// when the node is upgraded, and a 404 then fails it for a reason
    /// no log would explain.
    #[tokio::test]
    async fn a_challenge_survives_a_reopen() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("node.db");

        let database = crate::db::open(&path).await.expect("open");
        store_challenge(&database, "tok", "answer", "node.example.com")
            .await
            .expect("store");
        database.close().await.expect("close");

        let database = crate::db::open(&path).await.expect("reopen");
        assert_eq!(
            challenge_response(&database, "tok").await.expect("read"),
            Some("answer".to_string())
        );
    }

    #[tokio::test]
    async fn expired_challenges_are_swept() {
        let database = database().await;

        database
            .write(|connection| {
                connection.execute(
                    "INSERT INTO acme_challenge \
                       (\"token\", \"response\", \"domain\", \"expires_at\") \
                     VALUES ('old', 'x', 'a.example.com', 1)",
                    [],
                )?;
                Ok(())
            })
            .await
            .expect("insert");

        // Storing a new one sweeps the old.
        store_challenge(&database, "new", "y", "b.example.com")
            .await
            .expect("store");

        assert_eq!(
            challenge_response(&database, "old").await.expect("read"),
            None
        );
        assert!(challenge_response(&database, "new")
            .await
            .expect("read")
            .is_some());
    }

    /// Without a domain there is nothing a public CA could validate,
    /// and `ensure` must say so quietly rather than erroring.
    #[tokio::test]
    async fn without_a_domain_nothing_is_attempted() {
        let database = database().await;
        let resolver = certs::CertResolver::new();
        let config = Config::default();

        assert!(!ensure(&database, &config, &resolver).await.expect("ensure"));
    }

    #[tokio::test]
    async fn a_disabled_acme_is_not_attempted() {
        let database = database().await;
        let resolver = certs::CertResolver::new();
        let mut config = Config::default();
        config.node.domain = Some("node.example.com".into());
        config.acme.disabled = true;

        assert!(!ensure(&database, &config, &resolver).await.expect("ensure"));
    }

    /// A current ACME certificate must not trigger an order — that is
    /// what keeps the renewal loop from burning a rate limit.
    #[tokio::test]
    async fn a_fresh_certificate_is_left_alone() {
        let database = database().await;
        let resolver = certs::CertResolver::new();
        let mut config = Config::default();
        config.node.domain = Some("node.example.com".into());

        // A stored certificate that looks like ACME's and is not close
        // to expiring. `ensure` must return without talking to anyone —
        // if it tried, this test would hang or fail on the network.
        certs::save(
            &database,
            &StoredCert {
                domain: "node.example.com".into(),
                names: vec!["node.example.com".into()],
                cert_pem: "-----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----\n".into(),
                key_pem: String::new(),
                issuer: "acme".into(),
                not_after: now_ms() + 60 * 86_400_000,
            },
        )
        .await
        .expect("save");

        assert!(!ensure(&database, &config, &resolver).await.expect("ensure"));
    }

    #[test]
    fn the_directory_alias_resolves() {
        let mut config = Config::default();
        assert!(config.acme.directory_url().contains("acme-v02"));
        assert!(!config.acme.is_staging(), "production is the default");

        config.acme.directory = "staging".into();
        assert!(config.acme.is_staging());

        config.acme.directory = "https://example.test/dir".into();
        assert_eq!(config.acme.directory_url(), "https://example.test/dir");
        assert!(!config.acme.is_staging());
    }
}
