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
//! Bundled roots, as everywhere else in this binary. That means the
//! authority needs a publicly trusted certificate before it can enrol
//! anybody, which is a real limitation and the honest one: the
//! alternative is a joining node that accepts any certificate and hands
//! its bearer token to whoever answered.

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

#[derive(Debug, thiserror::Error)]
pub enum CallError {
    #[error("{0} is not an address this can call: {1}")]
    Address(String, String),
    // One paragraph, no line breaks: this reaches a terminal and a
    // note on a console page, and a message shaped for one of them
    // reads as damage in the other.
    #[error(
        "could not reach {0}: {1}. It has to be reachable at that address with a \
         certificate this machine trusts — a node serving its own certificate \
         cannot enrol anybody yet."
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
) -> Result<Accepted, CallError> {
    let url = format!("https://{endpoint}/api/network/join");
    match tokio::time::timeout(TIMEOUT, post(&url, secret, arriving)).await {
        Ok(result) => result,
        Err(_) => Err(CallError::TimedOut { url }),
    }
}

/// Ask an authority what it wants done.
pub async fn errands(
    endpoint: &str,
    secret: &str,
) -> Result<Vec<super::errand::Errand>, CallError> {
    let url = format!("https://{endpoint}/api/network/errands");
    match tokio::time::timeout(TIMEOUT, send(&url, secret, None)).await {
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
) -> Result<(), CallError> {
    let url = format!("https://{endpoint}/api/network/report");
    let body = serde_json::to_string(report).expect("a report is plain data");

    match tokio::time::timeout(TIMEOUT, send(&url, secret, Some(body))).await {
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
) -> Result<(), CallError> {
    let url = format!("https://{endpoint}/api/network/errands/{id}/done");
    let body =
        serde_json::to_string(&super::api::Outcome { error }).expect("an outcome is plain data");

    match tokio::time::timeout(TIMEOUT, send(&url, secret, Some(body))).await {
        Ok(result) => result.map(|_| ()),
        Err(_) => Err(CallError::TimedOut { url }),
    }
}

/// One request, with the bearer this node holds for that authority.
///
/// `None` is a GET, `Some` a POST — the two shapes this speaks, and
/// splitting them into two near-identical functions would be two places
/// for the TLS configuration to drift apart.
async fn send(url: &str, secret: &str, body: Option<String>) -> Result<Vec<u8>, CallError> {
    let (status, bytes) = round_trip(url, secret, body).await?;

    if status.is_success() {
        return Ok(bytes.to_vec());
    }
    match serde_json::from_slice::<Refusal>(&bytes) {
        Ok(refusal) => Err(CallError::Refused(url.into(), refusal.error)),
        Err(_) => Err(CallError::Unexpected(url.into(), status)),
    }
}

async fn post(url: &str, secret: &str, arriving: &Arriving) -> Result<Accepted, CallError> {
    let body = serde_json::to_string(arriving).expect("a node describes itself in plain data");
    let (status, bytes) = round_trip(url, secret, Some(body)).await?;
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
) -> Result<(StatusCode, hyper::body::Bytes), CallError> {
    let tls = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .build();
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
) -> Result<StatusCode, CallError> {
    let (Some(address), Some(ca)) = (&node.overlay_ip, &node.ca_pem) else {
        return Err(CallError::Address(
            node.id.clone(),
            "that node has no overlay address, or never sent its authority".into(),
        ));
    };
    // Lowercased here, and only here. A URI's host is normalised to
    // lower case by the `http` crate, so the name this client is *asked*
    // to resolve is never the mixed-case one an id produces — the first
    // version compared the two and every call died in `Connect` with the
    // reason swallowed. rustls compares DNS names case-insensitively, so
    // the certificate covering the mixed-case name still matches.
    let name = super::internal_name(&node.id).to_ascii_lowercase();
    let url = format!("https://{name}{path}");
    let socket: std::net::SocketAddr = format!("{address}:443")
        .parse()
        .map_err(|_| CallError::Address(node.id.clone(), format!("{address} is not an address")))?;

    // Parsed with the reader `edge::certs` already has, which is there
    // because `rustls-pemfile` would risk a second copy of the pki-types
    // that `CertificateDer` comes from.
    let anchors = crate::edge::certs::pem_certificates(ca)
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
    let request = builder
        .header(USER_AGENT, HeaderValue::from_static(AGENT))
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .body(body.unwrap_or_default())
        .map_err(|error| CallError::Address(url.clone(), error.to_string()))?;

    let response = tokio::time::timeout(OVERLAY_TIMEOUT, client.request(request))
        .await
        .map_err(|_| CallError::Unreachable(url.clone(), "it did not answer in time".into()))?
        .map_err(|error| CallError::Unreachable(url.clone(), error.to_string()))?;
    Ok(response.status())
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

    /// A node that is not there is the common failure, and the message
    /// has to name the certificate — that is what will actually be
    /// wrong, and it is not something the error from the socket says.
    #[tokio::test]
    async fn a_node_that_does_not_answer_says_what_to_check() {
        // Port 1 answers nothing, and reserved by RFC 6335 so nothing
        // on a developer's machine is listening there either.
        let error = announce("127.0.0.1:1", "a-secret", &arriving())
            .await
            .expect_err("nothing is there");

        let message = error.to_string();
        assert!(message.contains("could not reach"), "{message}");
        assert!(message.contains("certificate"), "{message}");
    }
}
