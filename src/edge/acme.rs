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
use super::policy;
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

/// How close to expiry a certificate nothing will replace may get
/// before the node signs for that name itself.
///
/// Three days rather than thirty: this is the floor, not the renewal
/// window. Dropping to a self-signed certificate is a visible
/// downgrade, and doing it a month early would throw away most of the
/// time somebody had to put a real one in place.
const FLOOR_DAYS: i64 = 3;

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
        // The directory URL itself, not a tag. Storing "acme" would
        // make a staging certificate and a production one look alike,
        // and then switching between them would silently keep serving
        // the old one — see `ensure`.
        issuer: config.acme.directory_url().to_string(),
        source: certs::Source::Acme,
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
/// Obtain or renew every certificate this node needs.
///
/// That is the node's own domain plus every hostname a service serves
/// HTTPS on. One certificate each rather than one with many names:
/// HTTP-01 cannot issue a wildcard, and a single multi-name
/// certificate would have to be reissued — and could fail entirely —
/// every time one service changes hostname.
///
/// Returns whether anything changed. A failure on one name does not
/// stop the others: a service with a broken DNS record must not keep
/// the node's own certificate from renewing.
/// Every name a public authority should answer for.
///
/// Read now, not at startup: an operator who fixed their DNS and set
/// the domain from the console needs the next pass to use it, not the
/// value this process booted with.
///
/// Its own function because the loop asks the same question before it
/// says it is working — an attempt over an empty list is not an
/// attempt, and a page that flashed "requesting" twice a day for a
/// node with no domain would be reporting a thing that never happened.
pub async fn wanted_names(database: &SqliteDatabase, config: &Config) -> Vec<String> {
    let mut names: Vec<String> = crate::node::settings::domain(database, config)
        .await
        .into_iter()
        .collect();
    match crate::platform::ports::all(database).await {
        Ok(ports) => names.extend(ports.into_iter().filter_map(|port| port.hostname)),
        Err(error) => tracing::warn!(%error, "could not read the service hostnames"),
    }
    // And the names this node serves for somebody else. They have no
    // port row here — the route arrived on an errand — so reading the
    // `port` table alone would leave the one thing this node was asked
    // to do answering with the local authority's certificate.
    match crate::network::claimed_for_others(database).await {
        Ok(claimed) => names.extend(claimed),
        Err(error) => tracing::warn!(%error, "could not read the names claimed for other nodes"),
    }
    // Sorted, because `dedup` only removes *adjacent* repeats and the
    // node's own domain can also be a service's hostname — which would
    // otherwise order two certificates for one name.
    names.sort();
    names.dedup();
    names
}

/// Keep the local authority's certificate covering every name the node
/// answers for, and put it in front of the listener.
///
/// Runs whether or not ACME is enabled, because it answers a different
/// question. A node that has just been given a domain is still
/// presenting a certificate for the name it had before — wrong name,
/// not merely untrusted — and on a node where ACME is switched off
/// that is the only certificate it will ever have.
///
/// `ensure_self_signed` already reissues when the name set changes, so
/// this is idempotent: a pass that changes nothing costs one read.
pub async fn refresh_local(
    database: &SqliteDatabase,
    config: &Config,
    resolver: &certs::CertResolver,
) -> AcmeResult<()> {
    let mut names = wanted_names(database, config).await;
    // The fallback answers a handshake that asked for no name, and
    // every certificate here is stored under it — see `CertResolver`.
    names.push(certs::FALLBACK_NAME.to_string());

    let before = certs::load(database, certs::FALLBACK_NAME).await?;
    let after = certs::ensure_self_signed(database, certs::FALLBACK_NAME, &names).await?;
    if before.map(|existing| existing.names) == Some(after.names) {
        return Ok(());
    }

    // Reloaded from storage rather than pushed in, for the same reason
    // as `ensure`: the resolver then serves exactly what a restart
    // would.
    resolver.replace(&certs::load_all(database).await?)?;
    tracing::info!(
        names = resolver.names().join(", "),
        "local certificate reissued"
    );
    Ok(())
}

/// Install whatever is on disk for `name`, if it is not already what
/// is being served.
///
/// This is the whole of "renew a certificate somebody gave us". The
/// node cannot ask for another — it has no relationship with whoever
/// signed it — but something else can keep the files current, and this
/// notices. Compared by content rather than by mtime: a file touched
/// without being changed is not a reason to swap the resolver, and a
/// file changed without its mtime moving still has to be picked up.
async fn install_from_file(
    database: &SqliteDatabase,
    resolver: &certs::CertResolver,
    name: &str,
    cert_path: &str,
    key_path: &str,
) -> AcmeResult<bool> {
    let found = certs::from_files(name, cert_path, key_path)?;

    if let Some(existing) = certs::load(database, name).await? {
        if existing.cert_pem == found.cert_pem && existing.key_pem == found.key_pem {
            return Ok(false);
        }
    }

    certs::save(database, &found).await?;
    resolver.replace(&certs::load_all(database).await?)?;
    tracing::info!(%name, %cert_path, "installed the certificate found on disk");
    Ok(true)
}

/// A certificate about to expire that nothing else will replace.
///
/// An expired certificate is a hard failure — no browser offers a way
/// past it — while a self-signed one is a warning somebody can click
/// through. So when a name is within [`FLOOR_DAYS`] of serving nothing
/// usable, the node signs for it rather than letting it lapse.
///
/// Only for sources that cannot replace themselves. ACME has its own
/// retry with backoff and taking over from it would throw away an
/// order that was about to succeed; a self-signed certificate is
/// already reissued by `refresh_local`. A file source is the case this
/// exists for: if whatever keeps those files fresh has stopped, no
/// amount of waiting produces a new one.
async fn hold_the_floor(
    database: &SqliteDatabase,
    config: &Config,
    resolver: &certs::CertResolver,
    name: &str,
    policy: &policy::Policy,
) -> AcmeResult<bool> {
    if policy.renew_with.is_self_serve() {
        return Ok(false);
    }
    let Some(existing) = certs::load(database, name).await? else {
        // Nothing installed at all: `refresh_local` covers the node
        // with a certificate carrying every name it answers for, so
        // this name is already being served something.
        return Ok(false);
    };
    if existing.not_after > now_ms() + FLOOR_DAYS * 86_400_000 {
        return Ok(false);
    }
    if existing.source == certs::Source::SelfSigned {
        // Already on the floor. Reissuing every pass would rewrite the
        // row twice a day and log it each time.
        return Ok(false);
    }

    tracing::warn!(
        %name,
        expires_in_days = (existing.not_after - now_ms()) / 86_400_000,
        "the certificate on disk is about to expire and nothing is replacing it; \
         signing for this name locally so it does not lapse"
    );
    certs::ensure_self_signed(database, name, &[name.to_string()]).await?;
    resolver.replace(&certs::load_all(database).await?)?;
    let _ = config;
    Ok(true)
}

pub async fn ensure_all(
    database: &SqliteDatabase,
    config: &Config,
    resolver: &certs::CertResolver,
) -> AcmeResult<bool> {
    let mut changed = false;
    let mut failure: Option<AcmeError> = None;

    for name in wanted_names(database, config).await {
        // What to do is read per name rather than inferred from what is
        // installed. Inferring it is what let this loop replace a
        // certificate it had not issued — see migration `0012`.
        let policy = policy::for_name(database, config, &name).await;
        let outcome = match &policy.renew_with {
            policy::RenewWith::Acme => ensure(database, config, resolver, &name).await,
            // Handled by `refresh_local`, which covers every name the
            // node answers for in one certificate. Reissuing here as
            // well would be a second certificate for the same name.
            policy::RenewWith::SelfSigned => Ok(false),
            policy::RenewWith::File {
                cert_path,
                key_path,
            } => install_from_file(database, resolver, &name, cert_path, key_path).await,
        };

        // Recorded against the name it is about. The node-wide
        // `acme_error` stays for `doctor` and for `install`, which ask
        // "did anything fail" rather than "what happened to this name".
        let recorded = match &outcome {
            Err(error) => policy::record_failure(database, &name, &error.to_string()).await,
            Ok(_) => policy::clear_failure(database, &name).await,
        };
        if let Err(error) = recorded {
            tracing::debug!(%name, %error, "could not record the certificate outcome");
        }

        match outcome {
            Ok(true) => changed = true,
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%name, %error, "could not obtain a certificate");
                failure.get_or_insert(error);
            }
        }

        // Whatever happened above, a name must not end up serving a
        // certificate that has expired. See `hold_the_floor`.
        match hold_the_floor(database, config, resolver, &name, &policy).await {
            Ok(true) => changed = true,
            Ok(false) => {}
            Err(error) => tracing::warn!(%name, %error, "could not hold the expiry floor"),
        }
    }

    match failure {
        // Reported only after everything that could work has been
        // tried, so one bad hostname costs the others nothing.
        Some(error) if !changed => Err(error),
        _ => Ok(changed),
    }
}

pub async fn ensure(
    database: &SqliteDatabase,
    config: &Config,
    resolver: &certs::CertResolver,
    domain: &str,
) -> AcmeResult<bool> {
    if config.acme.disabled {
        return Ok(false);
    }
    let domain = domain.to_string();

    let directory = config.acme.directory_url();
    if let Some(existing) = certs::load(database, &domain).await? {
        // The authority has to match, not merely be *an* authority.
        //
        // Found the hard way: a node tested against staging and then
        // switched to production kept serving the staging certificate,
        // because both were "acme" and it had not expired. Nothing
        // errored, nothing logged, and every browser rejected the site.
        // Comparing the directory URL makes the switch do what the
        // operator asked.
        let same_authority = existing.issuer == directory;
        let fresh = existing.not_after > now_ms() + RENEW_WITHIN_DAYS * 86_400_000;
        if same_authority && fresh {
            return Ok(false);
        }
        if !same_authority && existing.issuer != "self-signed" {
            tracing::info!(
                from = %existing.issuer,
                to = %directory,
                "the configured authority changed; reissuing"
            );
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
    wake: Arc<Wake>,
    cancel: wabot::lifecycle::Cancel,
) -> Result<(), std::convert::Infallible> {
    if config.acme.disabled {
        tracing::info!("ACME is disabled; serving the local authority's certificate");
    }

    // A node with no domain *yet* still runs this loop. It has nothing
    // to ask for until somebody sets one — and when they do, from the
    // console, this is what notices. Leaving early on "no domain at
    // startup" made that impossible without a restart.

    // Start at a minute and back off to six hours. The first attempts
    // are the ones most likely to be waiting on DNS the operator is
    // still setting up.
    let mut delay = std::time::Duration::from_secs(60);
    const MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);
    // Once a certificate is in hand, checking twice a day is plenty
    // against a 30-day window.
    const SETTLED_DELAY: std::time::Duration = std::time::Duration::from_secs(12 * 3600);

    loop {
        // Nothing to certify is not an attempt. Marking one anyway
        // would flash "requesting" on a page about a node that has no
        // domain, twice a day, for a request never made.
        let attempt = (!wanted_names(&database, &config).await.is_empty()).then(|| wake.begin());

        // Before the public one, and whether or not there will be a
        // public one. A node whose domain just changed is presenting a
        // certificate for the name it used to have; that is true even
        // where ACME is switched off, and it is the case somebody
        // testing on a laptop is most likely to be looking at.
        if let Err(error) = refresh_local(&database, &config, &resolver).await {
            tracing::warn!(%error, "could not reissue the local certificate");
        }

        if config.acme.disabled {
            drop(attempt);
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = wake.notified() => continue,
            }
        }

        match ensure_all(&database, &config, &resolver).await {
            Ok(changed) => {
                if changed {
                    tracing::info!(
                        names = resolver.names().join(", "),
                        "certificate installed without a restart"
                    );
                }
                if let Err(error) = crate::node::settings::set_acme_error(&database, None).await {
                    tracing::debug!(%error, "could not clear the last failure");
                }
                delay = SETTLED_DELAY;
            }
            Err(error) => {
                tracing::warn!(%error, retry_in = ?delay, "could not obtain a certificate");
                if let Err(error) = record_failure(&database, &error).await {
                    tracing::debug!(%error, "could not record the failure");
                }
                delay = (delay * 2).min(MAX_DELAY);
            }
        }

        // After the outcome is stored, never before: a page woken by
        // this reads the database, and waking it first would show it
        // the state the attempt just replaced.
        drop(attempt);

        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(delay) => {}
            // Somebody gave a service a hostname. Waiting out the
            // twelve-hour settled delay would leave that name without a
            // certificate for half a day, with the route already
            // pointing at it — the console would show it configured and
            // no browser could open it.
            _ = wake.notified() => {
                tracing::info!("certificates: something changed, checking now");
                // Back to the short delay: a name added a second ago is
                // exactly the case where DNS might still be settling,
                // and the backoff should start from the bottom again.
                delay = std::time::Duration::from_secs(60);
            }
        }
    }
}

/// How the rest of the node asks for a certificate check now, and how
/// a page watching one finds out that it finished.
///
/// A notification rather than a direct call: issuance belongs in the
/// loop, which owns the retries and the backoff. A console request
/// that issued inline would either block for the round trip to the
/// authority or invent a second retry policy beside this one.
///
/// The channel is the other direction, and it is why this is more than
/// a `Notify` now. Nothing used to record that an attempt was
/// *running* — only that one had failed — so the console could say
/// "asked for" and "failed" but never "asking", and the page that set
/// a domain had to tell the operator to reload in a few seconds.
#[derive(Debug)]
pub struct Wake {
    notify: tokio::sync::Notify,
    phase: tokio::sync::watch::Sender<Phase>,
}

/// Whether the loop is in the middle of an attempt.
///
/// Deliberately not the *outcome*: that is written to the database, and
/// two copies of it would be two things to keep in step. This says when
/// to go and read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Working,
}

impl Default for Wake {
    fn default() -> Self {
        Self {
            notify: tokio::sync::Notify::default(),
            phase: tokio::sync::watch::channel(Phase::Idle).0,
        }
    }
}

impl Wake {
    /// Ask the loop to look now. Never blocks; a wake with nothing to
    /// do costs one cheap pass over the stored certificates.
    pub fn now(&self) {
        self.notify.notify_one();
    }

    async fn notified(&self) {
        self.notify.notified().await
    }

    /// A signal every time the loop starts or stops working.
    pub fn watch(&self) -> tokio::sync::watch::Receiver<Phase> {
        self.phase.subscribe()
    }

    pub fn phase(&self) -> Phase {
        *self.phase.borrow()
    }

    /// Mark an attempt as running until the returned guard is dropped.
    ///
    /// A guard rather than a pair of calls: an early return or a panic
    /// between them would leave every page saying "requesting" for the
    /// rest of the process's life, and that is a lie no restart-free
    /// path could correct.
    fn begin(&self) -> Attempt<'_> {
        // `send_replace`, not `send`: with no page open there are no
        // receivers, and `send` reports that as an error rather than
        // storing the value the next subscriber should see.
        self.phase.send_replace(Phase::Working);
        Attempt(self)
    }
}

struct Attempt<'a>(&'a Wake);

impl Drop for Attempt<'_> {
    fn drop(&mut self) {
        self.0.phase.send_replace(Phase::Idle);
    }
}

/// Leave the reason where `doctor` and the console can show it,
/// without an operator reading the journal.
///
/// Beside the domain rather than on the certificate row, because the
/// failure that matters most — asked for a name, got nothing — is the
/// one where no row exists to carry it.
async fn record_failure(database: &SqliteDatabase, error: &AcmeError) -> AcmeResult<()> {
    crate::node::settings::set_acme_error(database, Some(&error.to_string())).await?;
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

    /// A name this node serves for another one has no port row here —
    /// the route arrived on an edge errand — so reading only the `port`
    /// table left it answering HTTPS with the local authority's
    /// certificate for ever, which is the one thing that node was asked
    /// not to do.
    #[tokio::test]
    async fn a_name_served_for_another_node_gets_a_certificate_too() {
        let database = database().await;
        let mut config = Config::default();
        config.node.domain = Some("node.example.com".into());

        crate::network::claim(&database, "app.example.com", Some("nd-elsewhere"))
            .await
            .expect("claim")
            .expect("granted");

        assert_eq!(
            wanted_names(&database, &config).await,
            vec![
                "app.example.com".to_string(),
                "node.example.com".to_string()
            ]
        );
    }

    /// The node's own domain can also be a service's hostname, and
    /// `dedup` only removes *adjacent* repeats — so an unsorted list
    /// ordered two certificates for one name.
    #[tokio::test]
    async fn one_name_is_wanted_once_however_many_places_it_appears() {
        let database = database().await;
        let mut config = Config::default();
        config.node.domain = Some("node.example.com".into());

        let project = crate::platform::projects::create(&database, "demo")
            .await
            .expect("project");
        let service =
            crate::platform::services::create(&database, &project.id, "web", "alpine:3.23", &[])
                .await
                .expect("service");
        crate::platform::ports::create(&database, &service.id, 80, false, Some("node.example.com"))
            .await
            .expect("port");

        assert_eq!(
            wanted_names(&database, &config).await,
            vec!["node.example.com".to_string()]
        );
    }

    /// Without a domain there is nothing a public CA could validate,
    /// and `ensure` must say so quietly rather than erroring.
    #[tokio::test]
    async fn without_a_domain_nothing_is_attempted() {
        let database = database().await;
        let resolver = certs::CertResolver::new();
        let config = Config::default();

        assert!(!ensure_all(&database, &config, &resolver)
            .await
            .expect("ensure"));
    }

    #[tokio::test]
    async fn a_disabled_acme_is_not_attempted() {
        let database = database().await;
        let resolver = certs::CertResolver::new();
        let mut config = Config::default();
        config.node.domain = Some("node.example.com".into());
        config.acme.disabled = true;

        assert!(!ensure_all(&database, &config, &resolver)
            .await
            .expect("ensure"));
    }

    /// A current ACME certificate must not trigger an order — that is
    /// what keeps the renewal loop from burning a rate limit.
    #[tokio::test]
    async fn a_fresh_certificate_is_left_alone() {
        let database = database().await;
        let resolver = certs::CertResolver::new();
        let mut config = Config::default();
        config.node.domain = Some("node.example.com".into());

        // A stored certificate from the configured authority, not
        // close to expiring. `ensure` must return without talking to
        // anyone — if it tried, this test would hang or fail on the
        // network.
        certs::save(&database, &stored_from(config.acme.directory_url(), 60))
            .await
            .expect("save");

        assert!(!ensure_all(&database, &config, &resolver)
            .await
            .expect("ensure"));
    }

    fn stored_from(issuer: &str, days_left: i64) -> StoredCert {
        StoredCert {
            domain: "node.example.com".into(),
            names: vec!["node.example.com".into()],
            cert_pem: "-----BEGIN CERTIFICATE-----\nQUJD\n-----END CERTIFICATE-----\n".into(),
            key_pem: String::new(),
            issuer: issuer.to_string(),
            not_after: now_ms() + days_left * 86_400_000,
            source: certs::Source::parse(issuer),
        }
    }

    /// The bug this test exists for, found on a real node: a node
    /// tested against staging and then switched to production kept
    /// serving the staging certificate, because both were tagged
    /// "acme" and it had not expired. Nothing errored and nothing
    /// logged; every browser rejected the site.
    ///
    /// `ensure` has to notice the authority changed. It cannot reach
    /// the network here, so "noticed" is observed as an error from the
    /// attempt rather than a silent `Ok(false)`.
    #[tokio::test]
    async fn switching_authority_does_not_keep_the_old_certificate() {
        let database = database().await;
        let resolver = certs::CertResolver::new();
        let mut config = Config::default();
        config.node.domain = Some("node.example.com".into());
        config.acme.directory = "production".into();

        let staging = "https://acme-staging-v02.api.letsencrypt.org/directory";
        certs::save(&database, &stored_from(staging, 60))
            .await
            .expect("save");

        // Production is configured and a *staging* certificate is
        // stored, so this must try to reissue. It fails — there is no
        // network in a test — and that failure is the evidence: the
        // early return would have been `Ok(false)`.
        let outcome = ensure_all(&database, &config, &resolver).await;
        assert!(
            outcome.is_err(),
            "a certificate from a different authority must not be kept: {outcome:?}"
        );
    }

    /// The same check must not reissue on every pass when nothing
    /// changed — that is what keeps the renewal loop off the rate
    /// limit.
    #[tokio::test]
    async fn the_same_authority_is_left_alone() {
        let database = database().await;
        let resolver = certs::CertResolver::new();
        let mut config = Config::default();
        config.node.domain = Some("node.example.com".into());
        config.acme.directory = "staging".into();

        certs::save(&database, &stored_from(config.acme.directory_url(), 60))
            .await
            .expect("save");

        assert!(!ensure_all(&database, &config, &resolver)
            .await
            .expect("ensure"));
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
