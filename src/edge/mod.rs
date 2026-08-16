//! The node's front door: one TLS listener, everything behind it.
//!
//! ## It is a router, not a second server
//!
//! The architecture sketch had this driving hyper's accept loop by
//! hand. It does not need to. `serve_on` already terminates TLS with a
//! dynamic certificate resolver, handles upgrades and drains
//! gracefully — so the edge is an ordinary `axum::Router` whose
//! **fallback** decides, per request, whether the control plane
//! answers or a container does.
//!
//! That deletes an accept loop, a graceful-shutdown implementation and
//! a connection tracker from this crate, and it means the edge cannot
//! drift from the rest of the framework on how a connection is served.
//!
//! ## Dispatch is on `Host`, not SNI
//!
//! SNI chooses the certificate, once per connection. Routing is per
//! *request*, because HTTP/2 lets one connection carry requests for
//! several hostnames — a client that coalesces two names onto one
//! connection would otherwise have both served by whichever it
//! happened to dial first.

pub mod acme;
pub mod certs;
pub mod health;
pub mod policy;
pub mod proxy;
pub mod routes;

use std::net::SocketAddr;
use std::sync::Arc;

use wabot::lifecycle::Cancel;
use wabot::rest::axum::extract::{Request, State};
use wabot::rest::axum::http::{header, HeaderValue, StatusCode};
use wabot::rest::axum::response::{IntoResponse, Response};
use wabot::rest::axum::Router;
use wabot::rest::{serve_on, RestServerConfig, TlsMode};
use wabot::sqlite::SqliteDatabase;

use certs::CertResolver;
use proxy::Proxy;
use routes::{RouteTable, Upstream};

/// Everything the edge needs on the request path.
#[derive(Clone)]
pub struct EdgeState {
    routes: Arc<RouteTable>,
    proxy: Proxy,
    /// The node's own API and console, invoked in-process.
    control_plane: Router,
    /// Whether an unmatched hostname reaches the control plane. See
    /// [`build`].
    default_to_control_plane: bool,
}

/// Build the edge and load its state from the database.
pub async fn build(
    database: &SqliteDatabase,
    control_plane: Router,
    config: &crate::config::Config,
    table: Arc<RouteTable>,
) -> anyhow::Result<(EdgeState, Arc<CertResolver>)> {
    // Every name this node can be reached by before ACME. `localhost`
    // is always present so a local `curl` works on a node with no DNS.
    // From the database, not the file: a domain set from the console
    // has to be the one this node presents a certificate for after a
    // restart, or the change lasts exactly until the next boot.
    let domain: Vec<String> = crate::node::settings::domain(database, config)
        .await
        .into_iter()
        .collect();
    // `own_names` adds the rest — the fallback, and the name every node has
    // whatever else it has. Through one function because the certificate
    // loop builds this same certificate from its own list, and whichever
    // ran last used to win.
    let names = certs::own_names(database, domain).await;
    certs::ensure_self_signed(database, certs::FALLBACK_NAME, &names).await?;

    let resolver = Arc::new(CertResolver::new());
    resolver.replace(&certs::load_all(database).await?)?;

    table.replace(routes::load_all(database).await?);

    // With no routes configured, every hostname reaches the control
    // plane. A fresh node has to be usable at whatever address the
    // operator can reach it on; refusing until they have written a
    // route would make the first request after `install` a 404.
    let default_to_control_plane = table.is_empty();
    if default_to_control_plane {
        tracing::info!("no routes configured; serving the control plane on every hostname");
    } else {
        tracing::info!(hosts = table.hosts().join(", "), "routes loaded");
    }

    Ok((
        EdgeState {
            routes: table,
            proxy: Proxy::new(),
            control_plane,
            default_to_control_plane,
        },
        resolver,
    ))
}

/// The HTTPS listener.
pub async fn serve_https(
    state: EdgeState,
    resolver: Arc<CertResolver>,
    bind: SocketAddr,
    cancel: Cancel,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let router = Router::new().fallback(dispatch).with_state(state);

    serve_on(
        listener,
        router,
        RestServerConfig::new(bind)
            .with_tls(TlsMode::Resolver(resolver))
            .with_shutdown(cancel),
    )
    .await
}

/// What the plain HTTP listener needs: where to redirect, and what to
/// answer a certificate authority with.
#[derive(Clone)]
pub struct HttpState {
    https_port: u16,
    database: Arc<SqliteDatabase>,
}

/// The plain HTTP listener: ACME challenges land here, everything else
/// is redirected.
///
/// Port 80 has to stay open even on an all-HTTPS node, and this is
/// why: HTTP-01 validation is a plaintext fetch, and a redirect to
/// HTTPS would send the authority to a certificate that does not exist
/// yet. The challenge route is therefore matched *before* the
/// redirecting fallback.
pub async fn serve_http(
    https_port: u16,
    database: Arc<SqliteDatabase>,
    bind: SocketAddr,
    cancel: Cancel,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let router = Router::new()
        .route(
            &format!("{}:token", acme::CHALLENGE_PREFIX),
            wabot::rest::axum::routing::get(acme_challenge),
        )
        .fallback(redirect_to_https)
        .with_state(HttpState {
            https_port,
            database,
        });

    serve_on(
        listener,
        router,
        RestServerConfig::new(bind).with_shutdown(cancel),
    )
    .await
}

/// Answer one HTTP-01 challenge.
///
/// Plain text, exactly the key authorization and nothing else: the
/// authority compares the body byte for byte, so a trailing newline or
/// a JSON wrapper fails the order.
async fn acme_challenge(
    State(state): State<HttpState>,
    wabot::rest::axum::extract::Path(token): wabot::rest::axum::extract::Path<String>,
) -> Response {
    match acme::challenge_response(&state.database, &token).await {
        Ok(Some(response)) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/plain")],
            response,
        )
            .into_response(),
        Ok(None) => {
            tracing::debug!(%token, "challenge asked for but not held");
            (StatusCode::NOT_FOUND, "unknown challenge").into_response()
        }
        Err(error) => {
            tracing::warn!(%error, "could not read the challenge");
            (StatusCode::INTERNAL_SERVER_ERROR, "storage error").into_response()
        }
    }
}

/// Per-request routing.
async fn dispatch(State(state): State<EdgeState>, request: Request) -> Response {
    let host = host_of(&request);

    let upstream = match host.as_deref().and_then(|host| state.routes.resolve(host)) {
        Some(upstream) => upstream,
        None if state.default_to_control_plane => Upstream::ControlPlane,
        None => {
            tracing::debug!(host = host.unwrap_or_default(), "no route");
            return not_found();
        }
    };

    match upstream {
        // `oneshot` rather than a request to ourselves over loopback:
        // the router is a `tower::Service`, so this is a function
        // call. No socket, no second serialization, no port.
        Upstream::ControlPlane => {
            use tower::ServiceExt;
            match state.control_plane.clone().oneshot(request).await {
                Ok(response) => response,
                // `Router`'s error type is `Infallible`; this arm
                // exists only to satisfy the signature.
                Err(never) => match never {},
            }
        }
        Upstream::Proxy(_) => {
            // Which copy takes it is the table's to decide — the
            // counter lives there, so two requests in a row do not both
            // land on the first replica.
            let Some(addr) = state.routes.next_upstream(&upstream) else {
                tracing::warn!("a route with nothing behind it");
                return not_found();
            };
            state.proxy.forward(addr, request).await
        }
    }
}

/// The `Host` a request is for.
///
/// HTTP/2 puts it in `:authority` and omits the header; HTTP/1.1 does
/// the opposite. Reading only one means the edge works on one protocol
/// version and silently 404s on the other.
fn host_of(request: &Request) -> Option<String> {
    request
        .uri()
        .authority()
        .map(|authority| authority.as_str().to_string())
        .or_else(|| {
            request
                .headers()
                .get(header::HOST)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        })
}

async fn redirect_to_https(State(state): State<HttpState>, request: Request) -> Response {
    let https_port = state.https_port;
    let Some(host) = host_of(&request) else {
        return (StatusCode::BAD_REQUEST, "missing Host").into_response();
    };
    // The port is rebuilt rather than carried over: the client
    // connected on the HTTP port, and sending them back to it loops.
    let name = host
        .rsplit_once(':')
        .map_or(host.as_str(), |(name, _)| name);
    let authority = if https_port == 443 {
        name.to_string()
    } else {
        format!("{name}:{https_port}")
    };
    let path = request
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or("/");

    let location = format!("https://{authority}{path}");
    match HeaderValue::from_str(&location) {
        // 308, not 302: the method and body have to survive, or a POST
        // quietly becomes a GET on the way to HTTPS.
        Ok(value) => (StatusCode::PERMANENT_REDIRECT, [(header::LOCATION, value)]).into_response(),
        Err(_) => (StatusCode::BAD_REQUEST, "bad Host").into_response(),
    }
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":{"message":"no application is configured for this hostname"}}"#,
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wabot::rest::axum::routing::get;
    use wabot::testing::RestHarness;

    fn control_plane() -> Router {
        Router::new().route("/whoami", get(|| async { "control-plane" }))
    }

    fn state(default_to_control_plane: bool) -> EdgeState {
        EdgeState {
            routes: Arc::new(RouteTable::new()),
            proxy: Proxy::new(),
            control_plane: control_plane(),
            default_to_control_plane,
        }
    }

    fn harness(state: EdgeState) -> RestHarness {
        RestHarness::new(Router::new().fallback(dispatch).with_state(state))
    }

    /// A fresh node has to answer on whatever address the operator can
    /// reach it on, or the first request after `install` is a 404.
    #[tokio::test]
    async fn with_no_routes_everything_reaches_the_control_plane() {
        let response = harness(state(true))
            .get("/whoami")
            .header("host", "anything.example.com")
            .send()
            .await;
        response.assert_ok();
        assert_eq!(response.body, "control-plane");
    }

    #[tokio::test]
    async fn once_routes_exist_an_unknown_host_is_refused() {
        let state = state(false);
        state
            .routes
            .replace([("node.example.com".to_string(), Upstream::ControlPlane)]);
        let harness = harness(state);

        harness
            .get("/whoami")
            .header("host", "node.example.com")
            .send()
            .await
            .assert_ok();

        let response = harness
            .get("/whoami")
            .header("host", "stranger.example.com")
            .send()
            .await;
        response.assert_status(StatusCode::NOT_FOUND);
        assert!(
            response.body.contains("no application is configured"),
            "the 404 says why: {}",
            response.body
        );
    }

    /// A container that is not listening must read as 502, not as a
    /// panic or a hang.
    #[tokio::test]
    async fn an_unreachable_upstream_is_a_bad_gateway() {
        let state = state(false);
        // Port 1 on loopback: reserved, and nothing will be there.
        state.routes.replace([(
            "app.example.com".to_string(),
            Upstream::Proxy(vec![SocketAddr::from(([127, 0, 0, 1], 1))]),
        )]);

        let response = harness(state)
            .get("/")
            .header("host", "app.example.com")
            .send()
            .await;
        response.assert_status(StatusCode::BAD_GATEWAY);
    }

    async fn http_state(https_port: u16) -> HttpState {
        HttpState {
            https_port,
            database: Arc::new(crate::db::open_in_memory().await.expect("open")),
        }
    }

    #[tokio::test]
    async fn the_redirect_preserves_the_path_and_uses_308() {
        let harness = RestHarness::new(
            Router::new()
                .fallback(redirect_to_https)
                .with_state(http_state(8443).await),
        );

        let response = harness
            .get("/deep/path")
            .header("host", "node.example.com:8080")
            .send()
            .await;

        response.assert_status(StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            response.header("location"),
            Some("https://node.example.com:8443/deep/path"),
            "the HTTP port must not survive into the redirect, or it loops"
        );
    }

    /// On the standard port the redirect must not spell it out —
    /// `https://host:443` is legal but reads as broken.
    #[tokio::test]
    async fn the_default_https_port_is_implicit() {
        let harness = RestHarness::new(
            Router::new()
                .fallback(redirect_to_https)
                .with_state(http_state(443).await),
        );
        let response = harness
            .get("/")
            .header("host", "node.example.com")
            .send()
            .await;
        assert_eq!(
            response.header("location"),
            Some("https://node.example.com/")
        );
    }

    /// The route the whole of ACME rests on. It must be matched
    /// *before* the redirecting fallback: HTTP-01 validation is a
    /// plaintext fetch, and a 308 to HTTPS would send the authority at
    /// a certificate that does not exist yet.
    #[tokio::test]
    async fn an_acme_challenge_is_answered_in_plain_http() {
        let database = Arc::new(crate::db::open_in_memory().await.expect("open"));
        let state = HttpState {
            https_port: 8443,
            database: database.clone(),
        };

        // Stage a challenge the way an order would.
        database
            .write(|connection| {
                connection.execute(
                    "INSERT INTO acme_challenge \
                       (\"token\", \"response\", \"domain\", \"expires_at\") \
                     VALUES ('tok-123', 'tok-123.keyauth', 'node.example.com', 99999999999999)",
                    [],
                )?;
                Ok(())
            })
            .await
            .expect("stage");

        let harness = RestHarness::new(
            Router::new()
                .route(
                    &format!("{}:token", acme::CHALLENGE_PREFIX),
                    wabot::rest::axum::routing::get(acme_challenge),
                )
                .fallback(redirect_to_https)
                .with_state(state),
        );

        let response = harness
            .get("/.well-known/acme-challenge/tok-123")
            .header("host", "node.example.com")
            .send()
            .await;

        response.assert_ok();
        assert_eq!(
            response.body, "tok-123.keyauth",
            "the authority compares this byte for byte"
        );

        // An unknown token is a 404, not a redirect: a redirect would
        // read to the authority as a broken challenge rather than an
        // absent one.
        let response = harness
            .get("/.well-known/acme-challenge/nope")
            .header("host", "node.example.com")
            .send()
            .await;
        response.assert_status(StatusCode::NOT_FOUND);

        // And everything else still redirects.
        let response = harness
            .get("/anything")
            .header("host", "node.example.com")
            .send()
            .await;
        response.assert_status(StatusCode::PERMANENT_REDIRECT);
    }

    // ---- the upgrade path, end to end ------------------------------
    //
    // `RestHarness` cannot reach this: it drives the router with
    // `oneshot`, and an upgrade is precisely the thing that outlives
    // the response. So these bind real sockets.
    //
    // No WebSocket library. The protocol being upgraded *to* is
    // irrelevant to a proxy — what matters is relaying the 101 and
    // then joining the two raw streams — so the upstream here speaks a
    // two-line echo protocol and the test asserts bytes come back.

    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};

    /// An upstream that answers 101 and then echoes.
    async fn echo_upstream() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");

        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut wants_upgrade = false;

                    // Read the request head.
                    loop {
                        let mut line = String::new();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        if line.to_ascii_lowercase().starts_with("upgrade:") {
                            wants_upgrade = true;
                        }
                        if line == "\r\n" || line == "\n" {
                            break;
                        }
                    }

                    let stream = reader.into_inner();
                    let (mut read, mut write) = tokio::io::split(stream);

                    if !wants_upgrade {
                        let _ = write
                            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\n\r\nplain")
                            .await;
                        return;
                    }

                    let _ = write
                        .write_all(
                            b"HTTP/1.1 101 Switching Protocols\r\n\
                              connection: upgrade\r\nupgrade: raw-echo\r\n\r\n",
                        )
                        .await;

                    let mut buffer = [0u8; 1024];
                    while let Ok(read_bytes) = read.read(&mut buffer).await {
                        if read_bytes == 0 || write.write_all(&buffer[..read_bytes]).await.is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });

        address
    }

    /// Start the edge on a real port, routing `app.example.com` at
    /// `upstream`.
    async fn edge_on_a_port(upstream: SocketAddr) -> (SocketAddr, Cancel) {
        let state = state(false);
        state.routes.replace([(
            "app.example.com".to_string(),
            Upstream::Proxy(vec![upstream]),
        )]);

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("addr");
        let cancel = Cancel::new();

        let router = Router::new().fallback(dispatch).with_state(state);
        let config = RestServerConfig::new(address).with_shutdown(cancel.clone());
        tokio::spawn(async move { serve_on(listener, router, config).await });

        (address, cancel)
    }

    /// The test M1 exists to pass: a WebSocket-shaped upgrade survives
    /// the proxy in both directions.
    #[tokio::test]
    async fn an_upgraded_connection_is_proxied_both_ways() {
        let upstream = echo_upstream().await;
        let (edge, cancel) = edge_on_a_port(upstream).await;

        let mut client = TcpStream::connect(edge).await.expect("connect");
        client
            .write_all(
                b"GET /socket HTTP/1.1\r\n\
                  host: app.example.com\r\n\
                  connection: Upgrade\r\n\
                  upgrade: raw-echo\r\n\r\n",
            )
            .await
            .expect("write");

        // The 101 has to come back through the edge.
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            let read = client.read(&mut byte).await.expect("read");
            assert_ne!(read, 0, "the edge closed before answering");
            head.push(byte[0]);
        }
        let head = String::from_utf8_lossy(&head);
        assert!(
            head.starts_with("HTTP/1.1 101"),
            "expected an upgrade, got: {head}"
        );

        // And then the streams have to actually be joined — this is
        // the half that a proxy can silently omit while still looking
        // like it works.
        client.write_all(b"ping").await.expect("write");
        let mut echoed = [0u8; 4];
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            client.read_exact(&mut echoed),
        )
        .await
        .expect("the upgraded stream carried data")
        .expect("read");
        assert_eq!(&echoed, b"ping");

        cancel.cancel();
    }

    /// The ordinary path over the same listener, so a failure in the
    /// upgrade test can be told apart from a broken proxy in general.
    #[tokio::test]
    async fn a_plain_request_is_proxied_too() {
        let upstream = echo_upstream().await;
        let (edge, cancel) = edge_on_a_port(upstream).await;

        let mut client = TcpStream::connect(edge).await.expect("connect");
        client
            .write_all(b"GET /thing HTTP/1.1\r\nhost: app.example.com\r\nconnection: close\r\n\r\n")
            .await
            .expect("write");

        let mut response = String::new();
        client.read_to_string(&mut response).await.expect("read");
        assert!(response.starts_with("HTTP/1.1 200"), "got: {response}");
        assert!(response.ends_with("plain"), "got: {response}");

        cancel.cancel();
    }
}
