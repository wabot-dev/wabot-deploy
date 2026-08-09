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

async fn post(url: &str, secret: &str, arriving: &Arriving) -> Result<Accepted, CallError> {
    let tls = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .build();
    let client = Client::builder(TokioExecutor::new()).build(tls);

    let body = serde_json::to_string(arriving).expect("a node describes itself in plain data");
    let bearer = HeaderValue::from_str(&format!("Bearer {secret}"))
        .map_err(|_| CallError::Address(url.into(), "that token cannot go in a header".into()))?;

    let request = Request::post(url)
        .header(USER_AGENT, HeaderValue::from_static(AGENT))
        .header(CONTENT_TYPE, HeaderValue::from_static("application/json"))
        .header(AUTHORIZATION, bearer)
        .body(body)
        .map_err(|error| CallError::Address(url.into(), error.to_string()))?;

    let response = client
        .request(request)
        .await
        .map_err(|error| CallError::Unreachable(url.into(), error.to_string()))?;

    let status = response.status();
    // Bounded before it is read, not after: a chunked body has no
    // length to check, and this node dialled an address out of a pasted
    // string. Reading it all and then measuring it is the same as not
    // measuring it.
    let bytes = http_body_util::Limited::new(response.into_body(), MAX_BODY)
        .collect()
        .await
        .map_err(|error| CallError::Unreadable(url.into(), error.to_string()))?
        .to_bytes();

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

#[cfg(test)]
mod tests {
    use super::*;

    fn arriving() -> Arriving {
        Arriving {
            node: "nd-abc".into(),
            name: "alpine".into(),
            public_key: "0hEr0DzTvMDTRfPPmYFCVCQ1cA0nnUnP+2fFqZBBBGQ=".into(),
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
