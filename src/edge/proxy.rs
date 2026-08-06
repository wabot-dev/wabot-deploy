//! Forwarding a request to a container on this node.
//!
//! ## Upgrades are the whole difficulty
//!
//! A reverse proxy that only forwards request and response bodies
//! looks finished and breaks every WebSocket. An upgrade needs
//! `hyper::upgrade::on` on **both** legs — the client's and the
//! upstream's — and the two raw streams copied between each other
//! after the 101 has been relayed. Getting the response back but not
//! joining the streams is the usual half-done version: the handshake
//! succeeds and then nothing is ever delivered.
//!
//! ## Plaintext upstream, on purpose
//!
//! The upstream is a container on the same host. TLS there would
//! encrypt a loopback hop against an attacker who, by definition,
//! already has the machine.

use std::net::SocketAddr;

use hyper::header::{HeaderValue, CONNECTION, HOST, UPGRADE};
use hyper::{Request, Response, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use wabot::rest::axum::body::Body;

/// Hop-by-hop headers, which belong to one connection and must not be
/// copied onto the next. RFC 9110 §7.6.1.
const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

#[derive(Clone)]
pub struct Proxy {
    client: Client<HttpConnector, Body>,
}

impl Default for Proxy {
    fn default() -> Self {
        Self::new()
    }
}

impl Proxy {
    pub fn new() -> Self {
        let mut connector = HttpConnector::new();
        // A container that is starting, or gone, should fail fast:
        // the caller is a browser waiting on a page, and thirty
        // seconds of nothing is worse than a prompt 502.
        connector.set_connect_timeout(Some(std::time::Duration::from_secs(5)));
        connector.set_nodelay(true);

        Self {
            client: Client::builder(TokioExecutor::new()).build(connector),
        }
    }

    /// Forward `request` to `upstream`.
    pub async fn forward(&self, upstream: SocketAddr, request: Request<Body>) -> Response<Body> {
        let wants_upgrade = upgrade_target(&request);

        match wants_upgrade {
            Some(protocol) => self.forward_upgrade(upstream, request, protocol).await,
            None => self.forward_plain(upstream, request).await,
        }
    }

    async fn forward_plain(&self, upstream: SocketAddr, request: Request<Body>) -> Response<Body> {
        let Some(outbound) = rewrite(upstream, request) else {
            return bad_gateway("bad upstream address");
        };

        match self.client.request(outbound).await {
            Ok(response) => {
                let (parts, body) = response.into_parts();
                let mut response = Response::from_parts(parts, Body::new(body));
                strip_hop_by_hop(response.headers_mut());
                response
            }
            Err(error) => {
                tracing::warn!(%upstream, %error, "upstream unreachable");
                bad_gateway("the application did not answer")
            }
        }
    }

    /// The upgrade path: relay the 101, then join the two raw streams.
    async fn forward_upgrade(
        &self,
        upstream: SocketAddr,
        mut request: Request<Body>,
        protocol: HeaderValue,
    ) -> Response<Body> {
        // Taken *before* the request is forwarded: after it is
        // consumed there is nothing left to take the upgrade from.
        let client_upgrade = hyper::upgrade::on(&mut request);

        let Some(mut outbound) = rewrite(upstream, request) else {
            return bad_gateway("bad upstream address");
        };
        // `rewrite` strips hop-by-hop headers, which is right for
        // every other request and exactly wrong for this one: the
        // upgrade *is* the hop-by-hop header.
        outbound
            .headers_mut()
            .insert(CONNECTION, HeaderValue::from_static("upgrade"));
        outbound.headers_mut().insert(UPGRADE, protocol);

        let mut response = match self.client.request(outbound).await {
            Ok(response) => response,
            Err(error) => {
                tracing::warn!(%upstream, %error, "upstream refused the upgrade");
                return bad_gateway("the application did not answer");
            }
        };

        if response.status() != StatusCode::SWITCHING_PROTOCOLS {
            // The upstream declined. Pass its answer through as-is —
            // it knows why, and inventing a 502 would hide it.
            let (parts, body) = response.into_parts();
            return Response::from_parts(parts, Body::new(body));
        }

        let upstream_upgrade = hyper::upgrade::on(&mut response);
        tokio::spawn(async move {
            match tokio::try_join!(client_upgrade, upstream_upgrade) {
                Ok((client, server)) => {
                    let mut client = hyper_util::rt::TokioIo::new(client);
                    let mut server = hyper_util::rt::TokioIo::new(server);
                    // Bidirectional until either side hangs up. This is
                    // the part that makes a WebSocket actually work.
                    if let Err(error) =
                        tokio::io::copy_bidirectional(&mut client, &mut server).await
                    {
                        tracing::debug!(%error, "upgraded connection ended");
                    }
                }
                Err(error) => tracing::warn!(%error, "upgrade handshake failed"),
            }
        });

        let (mut parts, _) = response.into_parts();
        // The 101 goes back with its own hop-by-hop headers intact:
        // this response *is* the connection change.
        parts.headers.remove(hyper::header::CONTENT_LENGTH);
        Response::from_parts(parts, Body::empty())
    }
}

/// Is this an upgrade request, and to what?
///
/// Both headers must agree: `Upgrade: websocket` without
/// `Connection: upgrade` is not an upgrade, and treating it as one
/// would hijack an ordinary request.
fn upgrade_target(request: &Request<Body>) -> Option<HeaderValue> {
    let connection = request.headers().get(CONNECTION)?.to_str().ok()?;
    let mentions_upgrade = connection
        .split(',')
        .any(|token| token.trim().eq_ignore_ascii_case("upgrade"));
    if !mentions_upgrade {
        return None;
    }
    request.headers().get(UPGRADE).cloned()
}

/// Point the request at the upstream and clean the headers.
/// Returns `None` only if the upstream address will not form a URI,
/// which a parsed `SocketAddr` cannot — the caller still handles it
/// rather than unwrapping, because "cannot happen" belongs in a
/// comment, not in a panic.
fn rewrite(upstream: SocketAddr, request: Request<Body>) -> Option<Request<Body>> {
    let (mut parts, body) = request.into_parts();

    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");
    let uri = match format!("http://{upstream}{path_and_query}").parse() {
        Ok(uri) => uri,
        Err(error) => {
            tracing::warn!(%error, "could not build the upstream URI");
            return None;
        }
    };
    parts.uri = uri;

    strip_hop_by_hop(&mut parts.headers);
    // The original Host is preserved: an application routing on it —
    // and many do — must see what the client asked for, not the
    // loopback address we happen to be dialling.
    if !parts.headers.contains_key(HOST) {
        if let Ok(value) = HeaderValue::from_str(&upstream.to_string()) {
            parts.headers.insert(HOST, value);
        }
    }

    Some(Request::from_parts(parts, body))
}

fn strip_hop_by_hop(headers: &mut hyper::HeaderMap) {
    // Whatever `Connection` lists is hop-by-hop too, by definition —
    // a proxy that only removes the fixed set leaks the rest.
    let listed: Vec<String> = headers
        .get(CONNECTION)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(',')
                .map(|token| token.trim().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default();

    for name in HOP_BY_HOP {
        headers.remove(*name);
    }
    for name in listed {
        headers.remove(name.as_str());
    }
}

fn bad_gateway(message: &str) -> Response<Body> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .header("content-type", "application/json")
        .body(Body::from(format!(
            "{{\"error\":{{\"message\":\"{message}\"}}}}"
        )))
        .expect("a constant response is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(headers: &[(&str, &str)]) -> Request<Body> {
        let mut builder = Request::builder().uri("http://example.com/path?q=1");
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        builder.body(Body::empty()).expect("request")
    }

    #[test]
    fn an_upgrade_needs_both_headers_to_agree() {
        assert!(upgrade_target(&request(&[
            ("connection", "Upgrade"),
            ("upgrade", "websocket")
        ]))
        .is_some());

        // A list, which is how browsers actually send it.
        assert!(upgrade_target(&request(&[
            ("connection", "keep-alive, Upgrade"),
            ("upgrade", "websocket")
        ]))
        .is_some());

        assert!(
            upgrade_target(&request(&[("upgrade", "websocket")])).is_none(),
            "Upgrade alone is not an upgrade"
        );
        assert!(
            upgrade_target(&request(&[("connection", "upgrade")])).is_none(),
            "and neither is Connection alone"
        );
        assert!(upgrade_target(&request(&[])).is_none());
    }

    #[test]
    fn the_upstream_uri_keeps_the_path_and_query() {
        let upstream = SocketAddr::from(([127, 0, 0, 1], 8080));
        let rewritten = rewrite(upstream, request(&[])).expect("rewrite");
        assert_eq!(
            rewritten.uri().to_string(),
            "http://127.0.0.1:8080/path?q=1"
        );
    }

    /// Copying these onto the next hop is the classic proxy bug: they
    /// describe *this* connection, not the message.
    #[test]
    fn hop_by_hop_headers_do_not_cross() {
        let upstream = SocketAddr::from(([127, 0, 0, 1], 8080));
        let rewritten = rewrite(
            upstream,
            request(&[
                ("connection", "keep-alive, x-custom-hop"),
                ("keep-alive", "timeout=5"),
                ("transfer-encoding", "chunked"),
                ("x-custom-hop", "secret"),
                ("x-forwarded-for", "1.2.3.4"),
            ]),
        )
        .expect("rewrite");

        for gone in [
            "connection",
            "keep-alive",
            "transfer-encoding",
            "x-custom-hop",
        ] {
            assert!(
                !rewritten.headers().contains_key(gone),
                "{gone} should not reach the upstream"
            );
        }
        assert!(
            rewritten.headers().contains_key("x-forwarded-for"),
            "an end-to-end header must survive"
        );
    }

    /// An application that routes on Host has to see what the client
    /// asked for, not the loopback address we dialled.
    #[test]
    fn the_clients_host_is_preserved() {
        let upstream = SocketAddr::from(([127, 0, 0, 1], 8080));
        let rewritten =
            rewrite(upstream, request(&[("host", "app.example.com")])).expect("rewrite");
        assert_eq!(
            rewritten.headers().get(HOST).unwrap(),
            "app.example.com",
            "the upstream sees the name the browser used"
        );
    }

    #[test]
    fn a_request_without_a_host_gets_the_upstream_as_one() {
        let upstream = SocketAddr::from(([127, 0, 0, 1], 8080));
        let rewritten = rewrite(upstream, request(&[])).expect("rewrite");
        assert_eq!(rewritten.headers().get(HOST).unwrap(), "127.0.0.1:8080");
    }
}
