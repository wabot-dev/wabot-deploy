//! The one thing this node fetches from the internet on purpose.
//!
//! A GET, over TLS, that follows redirects and reads the body into
//! memory. Small enough to be obvious, which is the point: the bytes
//! it returns are about to be checksummed and then executed, and a
//! client with a surprising behaviour is a bad place for that to
//! start.
//!
//! ## Why not a request client crate
//!
//! hyper is already here — the proxy is built on it — and reqwest
//! would bring a second TLS configuration, its own root store and a
//! connection pool this uses twice a month. The redirect loop below is
//! the only thing reqwest would have contributed.

use std::time::Duration;

use http_body_util::BodyExt;
use hyper::header::{HeaderValue, ACCEPT, LOCATION, USER_AGENT};
use hyper::{Request, StatusCode, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;

/// GitHub refuses requests without one, with a 403 that says nothing
/// about user agents.
const AGENT: &str = concat!("wabot-deploy/", env!("CARGO_PKG_VERSION"));

/// A release asset is tens of megabytes over a link this node does not
/// control. Long enough not to fail on a slow line, short enough that
/// a hung connection does not hold the update job open all day.
const TIMEOUT: Duration = Duration::from_secs(300);

/// Redirects to follow before giving up. GitHub sends one — the API to
/// the CDN — and a chain longer than this is a loop.
const MAX_REDIRECTS: usize = 5;

/// The largest body this will read.
///
/// The binary is around twenty megabytes; the ceiling exists so a
/// wrong URL cannot fill the node's memory before anything checks what
/// it downloaded.
const MAX_BODY: usize = 128 * 1024 * 1024;

pub type HttpResult<T> = Result<T, HttpError>;

#[derive(Debug, thiserror::Error)]
pub enum HttpError {
    #[error("{0} is not a URL: {1}")]
    Url(String, String),
    #[error("could not reach {0}: {1}")]
    Connect(String, String),
    #[error("{0} answered {1}")]
    Status(String, StatusCode),
    #[error("{0} redirected without saying where")]
    BadRedirect(String),
    #[error("{0} redirected more than {MAX_REDIRECTS} times")]
    TooManyRedirects(String),
    #[error("reading {0}: {1}")]
    Body(String, String),
    #[error("{0} is larger than {MAX_BODY} bytes")]
    TooLarge(String),
    #[error("{url} did not answer within {}s", TIMEOUT.as_secs())]
    TimedOut { url: String },
}

/// Fetch a URL, following redirects, and return the body.
///
/// `accept` is what GitHub uses to choose between the JSON *about* a
/// release and the release asset itself — the same asset URL serves
/// both, which is a fine way to download a JSON document when you
/// meant to download a binary.
pub async fn get(url: &str, accept: &str) -> HttpResult<Vec<u8>> {
    match tokio::time::timeout(TIMEOUT, follow(url, accept)).await {
        Ok(result) => result,
        Err(_) => Err(HttpError::TimedOut {
            url: url.to_string(),
        }),
    }
}

/// The same, as UTF-8 — for the API, which answers JSON.
pub async fn get_text(url: &str, accept: &str) -> HttpResult<String> {
    let bytes = get(url, accept).await?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn follow(url: &str, accept: &str) -> HttpResult<Vec<u8>> {
    // Bundled roots, and http1 only: GitHub speaks both, and one
    // protocol is one less thing to be wrong about a download that is
    // going to be executed.
    let tls = hyper_rustls::HttpsConnectorBuilder::new()
        .with_webpki_roots()
        .https_only()
        .enable_http1()
        .build();
    let client = Client::builder(TokioExecutor::new()).build(tls);

    let mut current = url.to_string();
    for _ in 0..MAX_REDIRECTS {
        let uri: Uri = current
            .parse()
            .map_err(|error: hyper::http::uri::InvalidUri| {
                HttpError::Url(current.clone(), error.to_string())
            })?;

        let request = Request::builder()
            .uri(uri)
            .header(USER_AGENT, HeaderValue::from_static(AGENT))
            .header(
                ACCEPT,
                HeaderValue::from_str(accept)
                    .map_err(|_| HttpError::Url(current.clone(), "bad Accept".into()))?,
            )
            .body(String::new())
            .map_err(|error| HttpError::Url(current.clone(), error.to_string()))?;

        let response = client
            .request(request)
            .await
            .map_err(|error| HttpError::Connect(current.clone(), error.to_string()))?;

        let status = response.status();
        if status.is_redirection() {
            let location = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| HttpError::BadRedirect(current.clone()))?;
            // Relative locations are legal and GitHub does not send
            // them; resolving one against the previous URL is a small
            // thing to get wrong, so it is refused rather than guessed.
            if !location.starts_with("http") {
                return Err(HttpError::BadRedirect(current));
            }
            current = location.to_string();
            continue;
        }

        if !status.is_success() {
            return Err(HttpError::Status(current, status));
        }

        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|error| HttpError::Body(current.clone(), error.to_string()))?
            .to_bytes();
        if body.len() > MAX_BODY {
            return Err(HttpError::TooLarge(current));
        }
        return Ok(body.to_vec());
    }

    Err(HttpError::TooManyRedirects(url.to_string()))
}
