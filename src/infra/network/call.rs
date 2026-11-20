//! The one call a joining node makes, and the only one that goes this
//! way round.
//!
//! Every other exchange in this design travels from the authority to
//! the node that granted it. This one is the exception that makes the
//! rest possible: after `join` has written the grant, the authority
//! still does not know the node exists. So the node says so, once,
//! authenticated by the secret out of the token it was just handed.
//!
//! ## Why not `update::http`
//!
//! That module downloads a release: it follows redirects, reads a
//! hundred megabytes, and its whole reason for existing is that the
//! bytes are about to be executed. This posts a few hundred bytes to
//! one URL, must not follow a redirect anywhere — a redirect here is
//! somebody sending the bearer token to a host that is not the one in
//! the token — and its failures need to name the certificate, because
//! that is the one that will actually happen.
//!
//! ## The certificate has to be trusted
//!
//! Bundled roots, as everywhere else in this binary, or the authority's
//! own certificate authority — never "accept anything", which would hand
//! the bearer token to whoever answered. The join knows that CA only by
//! the SHA-256 its token carries, and accepts a chain holding a CA with
//! that hash, verifying the leaf against it as against any root. The
//! answer hands the CA over, and every call after the join trusts the
//! public roots and that CA, for that authority's endpoint alone. That
//! is what lets two nodes with no public certificate — the two VMs —
//! join each other.

use std::time::Duration;

use http_body_util::BodyExt;
use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, USER_AGENT};
use hyper::{Request, StatusCode};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

use super::api::{Accepted, Arriving, Refusal};

const AGENT: &str = concat!("wabot-deploy/", env!("CARGO_PKG_VERSION"));

/// Long enough for a node on a slow line, short enough that somebody
/// watching `join` in a terminal does not conclude it hung.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A call over the overlay is a call to a machine one hop away with a
/// handshake already established, and it is made while somebody waits for
/// a page. Measured at 120 ms between the test nodes; anything past this
/// is a tunnel that is not working, and saying so quickly is worth more
/// than waiting for it.
const OVERLAY_TIMEOUT: Duration = Duration::from_secs(5);

/// Nothing this endpoint answers is large; a body past this is not a
/// wabot-deploy node.
const MAX_BODY: usize = 64 * 1024;

/// What an authority's certificate is believed by, beyond the public roots.
#[derive(Debug, Clone, Copy, Default)]
pub struct Trust<'a> {
    /// The SHA-256 of its CA, from the join token: the join, before the CA
    /// itself has arrived.
    pub pinned: Option<&'a str>,
    /// Its CA in PEM, kept since the join: every call after it.
    pub anchor: Option<&'a str>,
}

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("{0} is not an address this can call: {1}")]
    Address(String, String),
    // One paragraph, no line breaks: this reaches a terminal and a
    // note on a console page, and a message shaped for one of them
    // reads as damage in the other.
    #[error(
        "could not reach {0}: {1}. It has to be reachable at that address with a \
         certificate this machine trusts — a public one, or one signed by the \
         authority its token names."
    )]
    Unreachable(String, String),
    #[error("{url} did not answer within {}s", TIMEOUT.as_secs())]
    TimedOut { url: String },
    #[error("{0} refused the token: {1}")]
    Refused(String, String),
    #[error("{0} answered {1}, which is not something a node says")]
    Unexpected(String, StatusCode),
    #[error("{0} answered something this could not read: {1}")]
    Unreadable(String, String),
}

/// Tell `endpoint` that this node has joined it.
///
/// Convergent by design on the other side: the same node presenting the
/// same token again is the same join, so a `join` that is run twice —
/// or whose response was lost the first time — succeeds.
pub async fn announce(
    endpoint: &str,
    secret: &str,
    arriving: &Arriving,
    trust: Trust<'_>,
) -> Result<Accepted, CallError> {
    let url = format!("https://{endpoint}/api/network/join");
    match tokio::time::timeout(TIMEOUT, post(&url, secret, arriving, trust)).await {
        Ok(result) => result,
        Err(_) => Err(CallError::TimedOut { url }),
    }
}

/// Ask an authority what it wants done.
pub async fn errands(
    endpoint: &str,
    secret: &str,
    trust: Trust<'_>,
) -> Result<Vec<super::errand::Errand>, CallError> {
    let url = format!("https://{endpoint}/api/network/errands");
    match tokio::time::timeout(TIMEOUT, send(&url, secret, None, trust)).await {
        Ok(bytes) => {
            let bytes = bytes?;
            serde_json::from_slice(&bytes)
                .map_err(|error| CallError::Unreadable(url, error.to_string()))
        }
        Err(_) => Err(CallError::TimedOut { url }),
    }
}

/// Ask an authority who else is on its network.
///
/// Beside `errands` and on the same credential: the roster is a fact this
/// node is allowed to know because that node enrolled it. A failure here
/// costs nothing — the pass goes on to the errands, and the census is
/// asked for again in fifteen seconds.
pub async fn roster(
    endpoint: &str,
    secret: &str,
    trust: Trust<'_>,
) -> Result<super::roster::Roster, CallError> {
    let url = format!("https://{endpoint}/api/network/roster");
    match tokio::time::timeout(TIMEOUT, send(&url, secret, None, trust)).await {
        Ok(bytes) => {
            let bytes = bytes?;
            serde_json::from_slice(&bytes)
                .map_err(|error| CallError::Unreadable(url, error.to_string()))
        }
        Err(_) => Err(CallError::TimedOut { url }),
    }
}

/// Say what this node's copies of somebody else's services are doing.
///
/// Sent before collecting, so an authority queueing work has just heard
/// the truth about what is already there — and so a replica that was
/// evicted here stops being asked for in the same round trip.
pub async fn report(
    endpoint: &str,
    secret: &str,
    report: &super::api::Report,
    trust: Trust<'_>,
) -> Result<super::api::Heard, CallError> {
    let url = format!("https://{endpoint}/api/network/report");
    let body = serde_json::to_string(report).expect("a report is plain data");

    match tokio::time::timeout(TIMEOUT, send(&url, secret, Some(body), trust)).await {
        Ok(result) => serde_json::from_slice(&result?)
            .map_err(|error| CallError::Unreadable(url, error.to_string())),
        Err(_) => Err(CallError::TimedOut { url }),
    }
}

/// Ask an authority to do something.
///
/// The other direction, and the one the model did not have: collection
/// asks a node's authorities, so an errand *addressed to* one was asked
/// for by nobody. A joined node has always been able to dial its
/// authority — that is how it joined and how it reports — so the errand
/// goes the way everything else from this side goes.
///
/// The answer means "queued", not "done". What obeying it does happens
/// on that node's own pass, where every other errand it obeys is
/// carried out.
pub async fn ask(
    endpoint: &str,
    secret: &str,
    kind: &str,
    payload: &serde_json::Value,
    trust: Trust<'_>,
) -> Result<(), CallError> {
    let url = format!("https://{endpoint}/api/network/asked");
    let body = serde_json::to_string(&super::api::Asked {
        kind: kind.to_string(),
        payload: payload.clone(),
    })
    .expect("an errand is plain data");

    match tokio::time::timeout(TIMEOUT, send(&url, secret, Some(body), trust)).await {
        Ok(result) => result.map(|_| ()),
        Err(_) => Err(CallError::TimedOut { url }),
    }
}

/// Say how one went. `None` is success.
pub async fn settle(
    endpoint: &str,
    secret: &str,
    id: &str,
    error: Option<String>,
    trust: Trust<'_>,
) -> Result<(), CallError> {
    let url = format!("https://{endpoint}/api/network/errands/{id}/done");
    let body =
        serde_json::to_string(&super::api::Outcome { error }).expect("an outcome is plain data");

    match tokio::time::timeout(TIMEOUT, send(&url, secret, Some(body), trust)).await {
        Ok(result) => result.map(|_| ()),
        Err(_) => Err(CallError::TimedOut { url }),
    }
}

/// One request, with the bearer this node holds for that authority.
///
/// `None` is a GET, `Some` a POST — the two shapes this speaks, and
/// splitting them into two near-identical functions would be two places
/// for the TLS configuration to drift apart.
async fn send(
    url: &str,
    secret: &str,
    body: Option<String>,
    trust: Trust<'_>,
) -> Result<Vec<u8>, CallError> {
    let (status, bytes) = round_trip(url, secret, body, trust).await?;

    if status.is_success() {
        return Ok(bytes.to_vec());
    }
    match serde_json::from_slice::<Refusal>(&bytes) {
        Ok(refusal) => Err(CallError::Refused(url.into(), refusal.error)),
        Err(_) => Err(CallError::Unexpected(url.into(), status)),
    }
}

async fn post(
    url: &str,
    secret: &str,
    arriving: &Arriving,
    trust: Trust<'_>,
) -> Result<Accepted, CallError> {
    let body = serde_json::to_string(arriving).expect("a node describes itself in plain data");
    let (status, bytes) = round_trip(url, secret, Some(body), trust).await?;
    if status.is_success() {
        return serde_json::from_slice(&bytes)
            .map_err(|error| CallError::Unreadable(url.into(), error.to_string()));
    }

    // The reason the *other* node gave, when it gave one. A refusal
    // somebody can act on — "that token was already used" — is worth
    // far more than the status code it arrived with.
    match serde_json::from_slice::<Refusal>(&bytes) {
        Ok(refusal) => Err(CallError::Refused(url.into(), refusal.error)),
        Err(_) => Err(CallError::Unexpected(url.into(), status)),
    }
}

/// The wire, once. Everything above it decides what to say and what to
/// make of the answer; this is the only place the TLS and the bounds
/// are configured, so they cannot drift between the calls.
async fn round_trip(
    url: &str,
    secret: &str,
    body: Option<String>,
    trust: Trust<'_>,
) -> Result<(StatusCode, hyper::body::Bytes), CallError> {
    let tls = match (trust.pinned, trust.anchor) {
        (None, None) => hyper_rustls::HttpsConnectorBuilder::new()
            .with_webpki_roots()
            .https_only()
            .enable_http1()
            .build(),
        _ => hyper_rustls::HttpsConnectorBuilder::new()
            .with_tls_config(
                authority_config(trust).map_err(|reason| CallError::Address(url.into(), reason))?,
            )
            .https_only()
            .enable_http1()
            .build(),
    };
    let client = Client::builder(TokioExecutor::new()).build(tls);

    let bearer = HeaderValue::from_str(&format!("Bearer {secret}"))
        .map_err(|_| CallError::Address(url.into(), "that token cannot go in a header".into()))?;
    let builder = match body {
        Some(_) => Request::post(url),
        None => Request::get(url),
    };
    let request = builder
        .header(USER_AGENT, HeaderValue::from_static(AGENT))
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .header(AUTHORIZATION, bearer)
        .body(body.unwrap_or_default())
        .map_err(|error| CallError::Address(url.into(), error.to_string()))?;

    let response = client
        .request(request)
        .await
        .map_err(|error| CallError::Unreachable(url.into(), error.to_string()))?;
    let status = response.status();

    // Bounded before it is read, not after: a chunked body has no length
    // to check, and this node dialled an address out of a pasted string.
    // Reading it all and then measuring it is the same as not measuring.
    let bytes = http_body_util::Limited::new(response.into_body(), MAX_BODY)
        .collect()
        .await
        .map_err(|error| CallError::Unreadable(url.into(), error.to_string()))?
        .to_bytes();

    Ok((status, bytes))
}

fn authority_config(trust: Trust<'_>) -> Result<wabot::rest::rustls::ClientConfig, String> {
    use wabot::rest::rustls::{client::WebPkiServerVerifier, ClientConfig, RootCertStore};

    let mut public = RootCertStore::empty();
    public.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let public = WebPkiServerVerifier::builder(std::sync::Arc::new(public))
        .build()
        .map_err(|error| error.to_string())?;

    let anchored = match trust.anchor {
        None => None,
        Some(pem) => {
            let mut roots = RootCertStore::empty();
            for certificate in
                crate::infra::edge::certs::pem_certificates(pem).map_err(|e| e.to_string())?
            {
                roots.add(certificate).map_err(|error| error.to_string())?;
            }
            match roots.is_empty() {
                true => None,
                false => Some(
                    WebPkiServerVerifier::builder(std::sync::Arc::new(roots))
                        .build()
                        .map_err(|error| error.to_string())?,
                ),
            }
        }
    };

    Ok(ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(Authority {
            public,
            anchored,
            pinned: trust.pinned.map(str::to_ascii_lowercase),
        }))
        .with_no_client_auth())
}

/// The public roots, then the authority's own CA, then the CA its token
/// pinned — each a full verification of the leaf, name and dates and
/// signature, against a different root. Nothing here accepts a chain
/// that no root vouches for.
#[derive(Debug)]
struct Authority {
    public: std::sync::Arc<wabot::rest::rustls::client::WebPkiServerVerifier>,
    anchored: Option<std::sync::Arc<wabot::rest::rustls::client::WebPkiServerVerifier>>,
    pinned: Option<String>,
}

impl wabot::rest::rustls::client::danger::ServerCertVerifier for Authority {
    fn verify_server_cert(
        &self,
        end_entity: &wabot::rest::rustls::pki_types::CertificateDer<'_>,
        intermediates: &[wabot::rest::rustls::pki_types::CertificateDer<'_>],
        server_name: &wabot::rest::rustls::pki_types::ServerName<'_>,
        ocsp: &[u8],
        now: wabot::rest::rustls::pki_types::UnixTime,
    ) -> Result<wabot::rest::rustls::client::danger::ServerCertVerified, wabot::rest::rustls::Error>
    {
        let refused =
            match self
                .public
                .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
            {
                Ok(verified) => return Ok(verified),
                Err(error) => error,
            };
        if let Some(anchored) = &self.anchored {
            if let Ok(verified) =
                anchored.verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
            {
                return Ok(verified);
            }
        }
        let Some(pinned) = &self.pinned else {
            return Err(refused);
        };
        let Some(authority) = intermediates.iter().find(|certificate| {
            crate::infra::edge::certs::fingerprint_der(certificate.as_ref()) == *pinned
        }) else {
            return Err(refused);
        };
        let mut roots = wabot::rest::rustls::RootCertStore::empty();
        if roots.add(authority.clone().into_owned()).is_err() {
            return Err(refused);
        }
        wabot::rest::rustls::client::WebPkiServerVerifier::builder(std::sync::Arc::new(roots))
            .build()
            .map_err(|error| wabot::rest::rustls::Error::General(error.to_string()))?
            .verify_server_cert(end_entity, intermediates, server_name, ocsp, now)
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &wabot::rest::rustls::pki_types::CertificateDer<'_>,
        signature: &wabot::rest::rustls::DigitallySignedStruct,
    ) -> Result<
        wabot::rest::rustls::client::danger::HandshakeSignatureValid,
        wabot::rest::rustls::Error,
    > {
        self.public
            .verify_tls12_signature(message, certificate, signature)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &wabot::rest::rustls::pki_types::CertificateDer<'_>,
        signature: &wabot::rest::rustls::DigitallySignedStruct,
    ) -> Result<
        wabot::rest::rustls::client::danger::HandshakeSignatureValid,
        wabot::rest::rustls::Error,
    > {
        self.public
            .verify_tls13_signature(message, certificate, signature)
    }

    fn supported_verify_schemes(&self) -> Vec<wabot::rest::rustls::SignatureScheme> {
        self.public.supported_verify_schemes()
    }
}

/// Reach a node **over the overlay**, verified.
///
/// ## The premise this replaces
///
/// The module note above says the join is the only call that goes the
/// other way round, and the phases were built on a stronger claim:
/// nothing can dial *in* to a private node. That is true of the public
/// internet and false of the overlay — measured at 120 ms into a node
/// behind NAT with nothing forwarded, which is the same path the edge has
/// used since phase 7 to reach a container on that machine.
///
/// ## What makes it verified rather than merely encrypted
///
/// Two halves that are useless apart. The node is dialled by
/// [`super::internal_name`] — a name every node has, derived from its id,
/// needing no DNS because the address is supplied here — and the only
/// root accepted is the certificate authority that node presented when it
/// joined. So a private node with no domain, whose certificate no public
/// root will ever vouch for, is checked against the anchor it sent over a
/// channel this node had already authenticated.
///
/// Not `with_webpki_roots`, deliberately, and not "accept anything"
/// either: the first cannot work for a self-signed name and the second is
/// what a bearer token must never be handed to.
pub async fn to_node(
    node: &super::Node,
    path: &str,
    body: Option<String>,
    // A secret for the one thing being asked, when the endpoint needs
    // one. The overlay says the packets came from that peer and nothing
    // above it says who called, so an endpoint whose effect is not
    // harmless carries its own proof — see `api::challenge`.
    bearer: Option<&str>,
) -> Result<StatusCode, CallError> {
    let (Some(address), Some(ca)) = (&node.overlay_ip, &node.ca_pem) else {
        return Err(CallError::Address(
            node.id.clone(),
            "that node has no overlay address, or never sent its authority".into(),
        ));
    };
    let name = super::internal_name(&node.id);
    let url = format!("https://{name}{path}");
    let socket: std::net::SocketAddr = format!("{address}:443")
        .parse()
        .map_err(|_| CallError::Address(node.id.clone(), format!("{address} is not an address")))?;

    // Parsed with the reader `edge::certs` already has, which is there
    // because `rustls-pemfile` would risk a second copy of the pki-types
    // that `CertificateDer` comes from.
    let anchors = crate::infra::edge::certs::pem_certificates(ca)
        .map_err(|error| CallError::Address(node.id.clone(), error.to_string()))?;
    let mut roots = wabot::rest::rustls::RootCertStore::empty();
    for certificate in anchors {
        roots
            .add(certificate)
            .map_err(|error| CallError::Address(node.id.clone(), error.to_string()))?;
    }
    if roots.is_empty() {
        return Err(CallError::Address(
            node.id.clone(),
            "what that node sent as its authority holds no certificate".into(),
        ));
    }

    let tls = wabot::rest::rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_only()
        .enable_http1()
        // One name, one address. The overlay is where that node answers,
        // and a resolver that could reach anywhere else would be a name
        // this node trusts pointing at a machine it did not mean.
        .wrap_connector(fixed_connector(&name, socket));
    let client = Client::builder(TokioExecutor::new()).build(https);

    let builder = match body {
        Some(_) => Request::post(&url),
        None => Request::get(&url),
    };
    let mut builder = builder
        .header(USER_AGENT, HeaderValue::from_static(AGENT))
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(secret) = bearer {
        let value = HeaderValue::from_str(&format!("Bearer {secret}"))
            .map_err(|_| CallError::Address(url.clone(), "that secret is not a header".into()))?;
        builder = builder.header(AUTHORIZATION, value);
    }
    let request = builder
        .body(body.unwrap_or_default())
        .map_err(|error| CallError::Address(url.clone(), error.to_string()))?;

    let response = tokio::time::timeout(OVERLAY_TIMEOUT, client.request(request))
        .await
        .map_err(|_| CallError::Unreachable(url.clone(), "it did not answer in time".into()))?
        .map_err(|error| CallError::Unreachable(url.clone(), chain(&error)))?;
    Ok(response.status())
}

/// Ask a node something and read the answer.
///
/// [`to_node`] answers with a status and nothing else, which is all an
/// instruction needs — it is the *asking* that matters, and how it went
/// comes back another way. A question is the other shape: a log, a
/// reading, a list. The body is bounded, because what is on the other end
/// is a file somebody else's machine decides the size of.
///
/// The status comes back with it rather than being turned into an error:
/// a 403 from a node that has stopped consenting and a 404 for a replica
/// it no longer holds are both answers a page should print, and neither
/// is a failure of the call.
pub async fn ask_node(
    node: &super::Node,
    path: &str,
    body: Option<String>,
    bearer: Option<&str>,
) -> Result<(StatusCode, String), CallError> {
    let (Some(address), Some(ca)) = (&node.overlay_ip, &node.ca_pem) else {
        return Err(CallError::Address(
            node.id.clone(),
            "that node has no overlay address, or never sent its authority".into(),
        ));
    };
    let name = super::internal_name(&node.id);
    let url = format!("https://{name}{path}");
    let socket: std::net::SocketAddr = format!("{address}:443")
        .parse()
        .map_err(|_| CallError::Address(node.id.clone(), format!("{address} is not an address")))?;

    let client = client_for(&node.id, ca, &name, socket)?;
    let builder = match body {
        Some(_) => Request::post(&url),
        None => Request::get(&url),
    };
    let mut builder = builder
        .header(USER_AGENT, HeaderValue::from_static(AGENT))
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(secret) = bearer {
        let value = HeaderValue::from_str(&format!("Bearer {secret}"))
            .map_err(|_| CallError::Address(url.clone(), "that secret is not a header".into()))?;
        builder = builder.header(AUTHORIZATION, value);
    }
    let request = builder
        .body(body.unwrap_or_default())
        .map_err(|error| CallError::Address(url.clone(), error.to_string()))?;

    let response = tokio::time::timeout(OVERLAY_TIMEOUT, client.request(request))
        .await
        .map_err(|_| CallError::Unreachable(url.clone(), "it did not answer in time".into()))?
        .map_err(|error| CallError::Unreachable(url.clone(), chain(&error)))?;
    let status = response.status();
    let bytes = wabot::rest::axum::body::to_bytes(
        wabot::rest::axum::body::Body::new(response.into_body()),
        MAX_ANSWER,
    )
    .await
    .map_err(|error| CallError::Unreachable(url.clone(), error.to_string()))?;
    Ok((status, String::from_utf8_lossy(&bytes).into_owned()))
}

/// How much of an answer this node will hold in memory for a caller.
///
/// A log on a one-core node with 972 MB is not something to read whole
/// into a page, and the far side pages it anyway — this is the ceiling
/// that stops a wrong query being a memory problem here.
const MAX_ANSWER: usize = 1024 * 1024;

/// The client both of the above build: one root, one name, one address.
fn client_for(
    node_id: &str,
    ca: &str,
    name: &str,
    socket: std::net::SocketAddr,
) -> Result<
    Client<
        hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector<OneName>>,
        String,
    >,
    CallError,
> {
    let anchors = crate::infra::edge::certs::pem_certificates(ca)
        .map_err(|error| CallError::Address(node_id.to_string(), error.to_string()))?;
    let mut roots = wabot::rest::rustls::RootCertStore::empty();
    for certificate in anchors {
        roots
            .add(certificate)
            .map_err(|error| CallError::Address(node_id.to_string(), error.to_string()))?;
    }
    if roots.is_empty() {
        return Err(CallError::Address(
            node_id.to_string(),
            "what that node sent as its authority holds no certificate".into(),
        ));
    }

    let tls = wabot::rest::rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_tls_config(tls)
        .https_only()
        .enable_http1()
        .wrap_connector(fixed_connector(name, socket));
    Ok(Client::builder(TokioExecutor::new()).build(https))
}

/// Every reason, not just the outermost one.
///
/// hyper says `client error (Connect)` and keeps what actually happened —
/// a refused resolution, a certificate whose name did not match, a
/// handshake with no shared protocol — in the source chain. The outer
/// sentence alone sent this into two node runs looking for the wrong
/// thing, so the whole chain reaches the reader.
fn chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut reasons = vec![error.to_string()];
    let mut source = error.source();
    while let Some(next) = source {
        reasons.push(next.to_string());
        source = next.source();
    }
    reasons.join(": ")
}

/// An HTTP connector that resolves one name to one address.
fn fixed_connector(
    name: &str,
    address: std::net::SocketAddr,
) -> hyper_util::client::legacy::connect::HttpConnector<OneName> {
    let mut http = hyper_util::client::legacy::connect::HttpConnector::new_with_resolver(OneName {
        name: name.to_string(),
        address,
    });
    http.enforce_http(false);
    http
}

/// The resolver behind [`to_node`]: this name, that address, nothing else.
#[derive(Clone)]
pub struct OneName {
    name: String,
    address: std::net::SocketAddr,
}

impl tower::Service<hyper_util::client::legacy::connect::dns::Name> for OneName {
    type Response = std::vec::IntoIter<std::net::SocketAddr>;
    type Error = std::io::Error;
    type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn call(&mut self, asked: hyper_util::client::legacy::connect::dns::Name) -> Self::Future {
        let answer = match asked.as_str().eq_ignore_ascii_case(&self.name) {
            true => Ok(vec![self.address].into_iter()),
            // Refused rather than looked up. This client exists to reach
            // one node over the overlay; anything else it was asked for is
            // a bug that must not become a request to a stranger.
            false => {
                // Said out loud because hyper swallows it: a connector's
                // error reaches the caller as "client error (Connect)"
                // and nothing else, which is how the case mismatch above
                // cost a node run.
                tracing::debug!(asked = %asked, expected = %self.name, "refused to resolve");
                Err(std::io::Error::other(format!(
                    "{asked} is not the node this client was built for"
                )))
            }
        };
        std::future::ready(answer)
    }
}

#[cfg(test)]
/// A TLS server on loopback presenting what a node presents: a leaf
/// for `localhost` signed by its own authority, with the authority in
/// the chain. `answer` is handed its authority's CA and says the body
/// of every answer.
pub(crate) async fn loopback_authority(
    answer: impl Fn(&str) -> String + Send + 'static,
) -> (u16, String) {
    let database = crate::infra::db::open_in_memory().await.expect("open");
    let stored = crate::infra::edge::certs::ensure_self_signed(
        &database,
        "localhost",
        &["localhost".into()],
    )
    .await
    .expect("issued");
    let ca = crate::infra::edge::certs::ca_certificate_pem(&database)
        .await
        .expect("its authority");
    let config = std::sync::Arc::new(
        wabot::rest::rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                crate::infra::edge::certs::pem_certificates(&stored.cert_pem).expect("chain"),
                crate::infra::edge::certs::pem_private_key(&stored.key_pem).expect("key"),
            )
            .expect("a server"),
    );
    let body = answer(&ca);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("address").port();
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().flatten() {
            let Ok(connection) = wabot::rest::rustls::ServerConnection::new(config.clone()) else {
                continue;
            };
            let mut tls = wabot::rest::rustls::StreamOwned::new(connection, stream);
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                match tls.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
            }
            let _ = tls.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                     content-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
            tls.conn.send_close_notify();
            let _ = tls.flush();
        }
    });
    (port, ca)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arriving() -> Arriving {
        Arriving {
            node: "nd-abc".into(),
            name: "alpine".into(),
            public_key: "0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into(),
            accepted: None,
            ca: None,
        }
    }

    async fn an_authority_with_no_public_certificate() -> (u16, String) {
        super::loopback_authority(|_| "[]".to_string()).await
    }

    /// Two nodes with no public certificate join each other because the
    /// token pins the authority's CA. The pin vouches for that CA and
    /// nothing else: a wrong one, or none, is refused as an unknown issuer,
    /// and so is the right one at a name its leaf does not carry.
    #[tokio::test]
    async fn a_pinned_authority_is_trusted_and_nothing_else_is() {
        let (port, ca) = an_authority_with_no_public_certificate().await;
        let pin = crate::infra::edge::certs::fingerprint(&ca).expect("a fingerprint");
        let endpoint = format!("localhost:{port}");
        fn pinned(pin: &str) -> Trust<'_> {
            Trust {
                pinned: Some(pin),
                anchor: None,
            }
        }

        assert_eq!(
            errands(&endpoint, "secret", pinned(&pin))
                .await
                .expect("the pinned authority"),
            Vec::new()
        );
        assert_eq!(
            errands(&endpoint, "secret", pinned(&pin.to_ascii_uppercase()))
                .await
                .expect("hex is hex in either case"),
            Vec::new()
        );

        for (what, trust) in [
            ("no pin", Trust::default()),
            ("a wrong pin", pinned(&"0".repeat(64))),
        ] {
            let error = errands(&endpoint, "secret", trust).await.expect_err(what);
            assert!(
                matches!(error, CallError::Unreachable(..)),
                "{what}: {error}"
            );
        }

        let error = errands(&format!("127.0.0.1:{port}"), "secret", pinned(&pin))
            .await
            .expect_err("a name the leaf does not carry");
        assert!(matches!(error, CallError::Unreachable(..)), "{error}");
    }

    /// After the join the CA itself is kept, and every poll trusts it —
    /// the anchor, not the pin, because the leaf is reissued whenever a
    /// name is added and the CA is not.
    #[tokio::test]
    async fn every_call_after_the_join_trusts_the_authority_it_answered_with() {
        let (port, ca) = an_authority_with_no_public_certificate().await;
        let endpoint = format!("localhost:{port}");

        let anchored = Trust {
            pinned: None,
            anchor: Some(&ca),
        };
        assert_eq!(
            errands(&endpoint, "secret", anchored)
                .await
                .expect("anchored"),
            Vec::new()
        );

        let stranger = crate::infra::edge::certs::ca_certificate_pem(
            &crate::infra::db::open_in_memory().await.expect("open"),
        )
        .await
        .expect("another node's authority");
        let error = errands(
            &endpoint,
            "secret",
            Trust {
                pinned: None,
                anchor: Some(&stranger),
            },
        )
        .await
        .expect_err("another authority's CA vouches for nothing here");
        assert!(matches!(error, CallError::Unreachable(..)), "{error}");
    }

    /// A node that is not there is the common failure, and the message
    /// has to name the certificate — that is what will actually be
    /// wrong, and it is not something the error from the socket says.
    #[tokio::test]
    async fn a_node_that_does_not_answer_says_what_to_check() {
        // Port 1 answers nothing, and reserved by RFC 6335 so nothing
        // on a developer's machine is listening there either.
        let error = announce("127.0.0.1:1", "a-secret", &arriving(), Trust::default())
            .await
            .expect_err("nothing is there");

        let message = error.to_string();
        assert!(message.contains("could not reach"), "{message}");
        assert!(message.contains("certificate"), "{message}");
    }
}
