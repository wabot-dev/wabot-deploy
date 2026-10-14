//! What every console page is drawn from: the node's handles, the
//! secrets waiting to be shown once, and the certificate facts more
//! than one page reads.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::sqlite::SqliteDatabase;

use crate::infra::config::Config;
use crate::infra::edge::certs;

/// What every console page reads from. Never cached: this is a status
/// console, and a cached status is the thing it exists not to be.
pub struct ConsoleState {
    pub(crate) database: Arc<SqliteDatabase>,
    pub(crate) config: Config,
    pub(crate) deployer: Arc<crate::infra::deploy::Deployer>,
    /// What releases exist, as last read from GitHub. Shared so that
    /// the page and the update it starts agree about what they are
    /// installing.
    pub(crate) catalogue: Arc<crate::infra::update::Catalogue>,
    /// Secrets waiting for the one page that will show them.
    pub(crate) reveals: Reveals,
    /// The certificate loop, to ask it to look now and to watch what
    /// it is doing. Held here as well as inside the deployer because
    /// the node page reports on it, which is not deploying.
    pub(crate) certificates: Arc<crate::infra::edge::acme::Wake>,
    /// For enqueuing work. `run_command` is a free function over the
    /// container — the framework's reasoning is that an `Async`
    /// singleton would have to hold a container that holds it — so the
    /// container is what a caller needs. It is a cheap clonable handle.
    pub(crate) container: Container,
}

/// A secret handed from a POST to the GET that follows it.
///
/// A push token exists in clear for exactly the moment it is minted,
/// and the page that shows it is reached by redirect. It used to travel
/// there in the query string — which put it in the address bar, in the
/// browser's history, and back on the screen on every refresh. This
/// console's own rule is that a query parameter is fine *because it is
/// not secret* (see `back_with_error`), and that one was the exception
/// nobody noticed.
///
/// So the redirect now carries a nonce that names the secret, and the
/// nonce is spent when the page reads it. In memory rather than in the
/// database because the plaintext must not outlive the process, and a
/// restart losing an unread secret is the right failure — the token is
/// revocable and another one costs a click.
#[derive(Default)]
pub(crate) struct Reveals {
    held: std::sync::Mutex<std::collections::HashMap<String, (String, i64)>>,
}

/// How long a minted secret waits to be read. Long enough for a
/// redirect, short enough that a browser left on the login screen does
/// not keep one in memory all afternoon.
const REVEAL_TTL_MS: i64 = 60_000;

impl Reveals {
    /// Hold a secret, returning the nonce that names it.
    pub(crate) fn stash(&self, secret: String) -> String {
        let nonce = wabot::prelude::password::generate(24);
        let now = now_ms();
        let mut held = self.held.lock().expect("no panic holds this lock");
        // Swept here rather than on a timer: the only way the map
        // grows is somebody minting a token, so the act of growing it
        // is the right moment to drop what expired.
        held.retain(|_, (_, at)| now - *at < REVEAL_TTL_MS);
        held.insert(nonce.clone(), (secret, now));
        nonce
    }

    /// Read a secret, once.
    pub(crate) fn take(&self, nonce: &str) -> Option<String> {
        let now = now_ms();
        let mut held = self.held.lock().expect("no panic holds this lock");
        held.remove(nonce)
            .filter(|(_, at)| now - *at < REVEAL_TTL_MS)
            .map(|(secret, _)| secret)
    }
}

impl ConsoleState {
    /// Where a link should send somebody, for the frontier they belong
    /// to: a link somebody pastes somewhere else, so never `localhost`,
    /// which only the node can open. With no domain anywhere it is the
    /// address `infra::node::authority` gives — the fallback was
    /// `localhost`, and an invitation from a VM node said exactly that.
    ///
    /// **An invitation into an organisation has to land on that
    /// organisation's own name.** Sent to the node's, the person signs
    /// up and is bounced on their first click: their account is scoped
    /// to the organisation, the `Host` says the node, and the session
    /// middleware refuses a session whose scope does not match the door
    /// it was presented at. A working link that leads nowhere is worse
    /// than a broken one, because it looks like their fault.
    ///
    /// Falls back to the node's own name for an organisation with no
    /// domain yet. That link works — the account is made — and the
    /// console they belong to does not exist until somebody gives them
    /// one, which is a thing the page says rather than something this
    /// can fix.
    pub(crate) async fn base_url_for(&self, tenant_id: Option<&str>) -> String {
        let theirs = match tenant_id {
            Some(id) => crate::infra::repositories::tenants::find(&self.database, id)
                .await
                .ok()
                .flatten()
                .and_then(|tenant| tenant.domain),
            None => None,
        };
        let domain = match theirs {
            Some(domain) => Some(domain),
            None => crate::infra::node::settings::domain(&self.database).await,
        };
        format!(
            "https://{}",
            crate::infra::node::authority(domain.as_deref(), self.config.edge.https_port)
        )
    }

    pub fn new(
        container: Container,
        database: Arc<SqliteDatabase>,
        config: Config,
        routes: Arc<crate::infra::edge::routes::RouteTable>,
        certificates: Arc<crate::infra::edge::acme::Wake>,
    ) -> Self {
        let deployer = Arc::new(
            crate::infra::deploy::Deployer::new(database.clone(), &config)
                .with_routes(routes)
                .with_certificates(certificates.clone()),
        );
        Self {
            database,
            config,
            deployer,
            // The process's one catalogue, not a second of its own: the
            // collector installs releases too, and two caches are two
            // answers to "what is v0.7.1" and twice the calls to a host
            // that allows sixty an hour.
            catalogue: crate::infra::update::shared(),
            reveals: Reveals::default(),
            certificates,
            container,
        }
    }
}

pub(crate) use crate::shell::dtos::certificate_facts::CertificateFacts;

pub(crate) async fn certificate_facts(state: &ConsoleState) -> CertificateFacts {
    // The domain is not carried back out: every caller already knows it
    // — they render the form that sets it — and two copies of a value
    // one page shows twice is how they end up disagreeing.
    match crate::infra::node::settings::domain(&state.database).await {
        Some(domain) => certificate_facts_for(state, &domain).await,
        None => CertificateFacts::none(),
    }
}

/// The same, for any name the node answers for.
///
/// A service hostname has a certificate of its own, with its own
/// source and its own expiry — the node's domain is not a special case
/// of anything, it is just the name that had a page first.
pub(crate) async fn certificate_facts_for(state: &ConsoleState, name: &str) -> CertificateFacts {
    let stored = certs::load(&state.database, name).await.ok().flatten();

    match stored {
        Some(certificate) => {
            let issuer = short_issuer(&certificate.issuer);
            CertificateFacts {
                days_left: (certificate.not_after - now_ms()) / 86_400_000,
                // Staging roots are untrusted by design, and saying so
                // here saves somebody working out why their browser
                // still complains.
                //
                // A certificate found on disk is not claimed either
                // way: whether a browser trusts it depends on a trust
                // store this node cannot see, and guessing would be the
                // console asserting something it does not know.
                trusted: issuer == "letsencrypt",
                issuer,
                source: certificate.source,
            }
        }
        None => CertificateFacts::none(),
    }
}

fn short_issuer(issuer: &str) -> String {
    match issuer {
        "self-signed" => "self-signed".into(),
        url if url.contains("acme-staging-v02.api.letsencrypt.org") => "letsencrypt-staging".into(),
        url if url.contains("acme-v02.api.letsencrypt.org") => "letsencrypt".into(),
        url => url.split('/').nth(2).unwrap_or(url).into(),
    }
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}
