//! Blobs: the layers and configs a push is mostly made of.
//!
//! ## The upload session is containerd's, not ours
//!
//! An OCI upload is resumable: the client POSTs to start, PATCHes
//! bytes across as many requests as it likes, and PUTs to finish. The
//! obvious implementation keeps a file or an open gRPC stream per
//! upload in a map — and leaks one every time a client walks away
//! mid-push.
//!
//! containerd's content service already models exactly this: a write
//! is a named transaction, resumable by ref and offset, and it cleans
//! up its own abandoned ones. So the session id *is* the ref, and this
//! module holds no state at all between requests.

use wabot::rest::axum::body::Body;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::http::{header, StatusCode};
use wabot::rest::axum::response::Response;
use wabot::rest::{RestError, RestResult};

use std::time::Duration;

use crate::infra::runtime::client::{ClientError, ClientResult, Containerd};
use crate::infra::runtime::{content, leases};

/// How much of one request's body is held in memory at a time.
///
/// A layer is hundreds of megabytes and the node has hundreds in
/// total, so the body is read in pieces and each is handed to
/// containerd before the next is read. A megabyte, not four: a client
/// pushes several layers at once, and each of them holding four is
/// most of a small node's headroom.
const CHUNK: usize = 1024 * 1024;

/// How long an upload may send nothing before it is given up on.
///
/// A client that stops sending without hanging up held its connection
/// and everything it had sent for as long as it stayed silent — ninety
/// seconds measured on the VM, with no sign of ending. Sixty is nginx's
/// `client_body_timeout`, which is what anybody pushing through a proxy
/// already lives with.
const IDLE: Duration = Duration::from_secs(60);

/// `HEAD /v2/<name>/blobs/<digest>` — do we already have it?
pub async fn head(digest: &str) -> RestResult<Response> {
    let client = connect().await?;
    match content::exists(&client, digest).await.map_err(internal)? {
        Some(size) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_LENGTH, size)
            .header("docker-content-digest", digest)
            .body(Body::empty())
            .expect("a constant response is well-formed")),
        None => Ok(missing()),
    }
}

/// `GET /v2/<name>/blobs/<digest>`.
pub async fn get(digest: &str) -> RestResult<Response> {
    let client = connect().await?;
    let Some(size) = content::exists(&client, digest).await.map_err(internal)? else {
        return Ok(missing());
    };
    let chunks = content::stream(&client, digest).await.map_err(internal)?;
    Ok(serving(digest, size, chunks))
}

/// A layer is hundreds of megabytes and the node's spare memory often less,
/// so the body is the store's own stream: whole, the 674 MB layer another
/// node asked node-0 for was more than the 481 MB it had free.
fn serving<S>(digest: &str, size: i64, chunks: S) -> Response
where
    S: futures_util::Stream<Item = ClientResult<Vec<u8>>> + Send + 'static,
{
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, size)
        .header("docker-content-digest", digest)
        .body(Body::from_stream(chunks))
        .expect("a constant response is well-formed")
}

/// `POST /v2/<name>/blobs/uploads/` — begin.
///
/// Also handles the monolithic form, where the whole blob and its
/// digest arrive with the POST. Clients use it for small blobs, and a
/// registry that only implements the long form makes them do three
/// round trips for a two-kilobyte config.
pub async fn start(name: &str, request: Request) -> RestResult<Response> {
    let digest = query_digest(&request);
    let session = session_id();

    match digest {
        Some(digest) => {
            let client = leased(&connect().await?, &session).await?;
            write_and_commit(&client, name, &session, 0, &digest, request).await
        }
        None => Ok(accepted(name, &session, 0)),
    }
}

/// `PATCH` — a chunk.
pub async fn patch(name: &str, session: &str, request: Request) -> RestResult<Response> {
    let range_start = content_range_start(&request);
    let client = leased(&connect().await?, session).await?;
    let offset = starting_offset(&client, session, range_start).await?;
    let end = content::write_chunks(&client, session, offset, pieces(request))
        .await
        .map_err(refused)?;
    Ok(accepted(name, session, end))
}

/// `PUT ?digest=…` — the last chunk, and the commit.
pub async fn finish(name: &str, session: &str, request: Request) -> RestResult<Response> {
    let Some(digest) = query_digest(&request) else {
        return Err(RestError::Client {
            status: 400,
            message: "an upload is finished with ?digest=".into(),
        });
    };

    let range_start = content_range_start(&request);
    let client = leased(&connect().await?, session).await?;
    let offset = starting_offset(&client, session, range_start).await?;
    write_and_commit(&client, name, session, offset, &digest, request).await
}

/// **The body streams, the way a `PATCH` does.** A client is free to
/// send a whole layer in the `PUT` (or the monolithic `POST`) and never
/// `PATCH` at all. BuildKit does: a `POST`, then one `PUT` with the
/// layer. Read into memory under a limit, that refused the first layer
/// of a real image with "length limit exceeded" — found pushing from a
/// GitHub Actions runner, while the same image pushed fine from Docker,
/// which `PATCH`es.
///
/// A failure aborts the upload. What a failed request wrote is not the
/// blob, and a client that sends the blob again to the same session
/// would otherwise land after it.
async fn write_and_commit(
    client: &Containerd,
    name: &str,
    session: &str,
    offset: i64,
    digest: &str,
    request: Request,
) -> RestResult<Response> {
    let written = content::write_and_commit(
        client,
        session,
        offset,
        pieces(request),
        digest,
        Default::default(),
    )
    .await;
    let Err(error) = written else {
        return Ok(created(name, digest));
    };
    if let Err(abort) = content::abort(client, session).await {
        tracing::warn!(%session, %abort, "an upload that failed was left behind");
    }
    commit_refused(error, digest)
}

/// containerd refuses a commit whose bytes do not hash to the digest the
/// client named with `FailedPrecondition`, and that is the client's
/// mistake, not the node's: a 500 sent people to the node's journal for
/// what `DIGEST_INVALID` would have told them, and is what a client
/// prints.
fn commit_refused(error: ClientError, digest: &str) -> RestResult<Response> {
    match error {
        ClientError::Call { source, .. } if source.code() == tonic::Code::FailedPrecondition => {
            Ok(digest_invalid(digest, source.message()))
        }
        other => Err(refused(other)),
    }
}

fn pieces(request: Request) -> impl futures_util::Stream<Item = std::io::Result<Vec<u8>>> {
    coalesced(request.into_body().into_data_stream(), IDLE)
}

/// A body arrives in whatever sizes the network chose — often a few
/// kilobytes — and one gRPC message each would be tens of thousands of
/// them for one layer.
///
/// A body that stops arriving ends in an error rather than just ending:
/// what came before it is not the whole blob, and committing it as one
/// tells the client about a digest when what happened is that its
/// connection broke. So does one that sends nothing for `idle`.
fn coalesced<S, B, E>(
    body: S,
    idle: Duration,
) -> impl futures_util::Stream<Item = std::io::Result<Vec<u8>>>
where
    S: futures_util::Stream<Item = Result<B, E>>,
    B: AsRef<[u8]>,
    E: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    async_stream::stream! {
        use futures_util::StreamExt;
        futures_util::pin_mut!(body);

        let mut buffered: Vec<u8> = Vec::with_capacity(CHUNK);
        loop {
            let next = match tokio::time::timeout(idle, body.next()).await {
                Ok(Some(next)) => next,
                Ok(None) => break,
                Err(_) => {
                    yield Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        format!("nothing arrived for {} seconds", idle.as_secs()),
                    ));
                    return;
                }
            };
            match next {
                Ok(bytes) => {
                    buffered.extend_from_slice(bytes.as_ref());
                    if buffered.len() >= CHUNK {
                        yield Ok(std::mem::replace(&mut buffered, Vec::with_capacity(CHUNK)));
                    }
                }
                Err(error) => {
                    yield Err(std::io::Error::other(error));
                    return;
                }
            }
        }
        if !buffered.is_empty() {
            yield Ok(buffered);
        }
    }
}

fn refused(error: ClientError) -> RestError {
    match error {
        ClientError::Interrupted(error) => RestError::Client {
            status: 400,
            message: format!("could not read the body: {error}"),
        },
        other => internal(other),
    }
}

pub(crate) async fn read_body(request: Request) -> RestResult<Vec<u8>> {
    // A manifest is kilobytes and a config is smaller. This is not the
    // path a layer takes — that one streams.
    const MAX: usize = 32 * 1024 * 1024;

    wabot::rest::axum::body::to_bytes(request.into_body(), MAX)
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| RestError::Client {
            status: 400,
            message: format!("could not read the body: {error}"),
        })
}

pub(crate) fn session_id() -> String {
    format!("wd-{}", wabot::prelude::password::generate(20))
}

/// `client`, with everything it writes held under the lease `id`.
///
/// A committed blob nothing references is the collector's at its next run,
/// and a node with a database runs one a minute — so a push's first layers
/// were gone before its manifest arrived to name them (node-0, 2026-11-14).
pub(crate) async fn leased(client: &Containerd, id: &str) -> RestResult<Containerd> {
    leases::hold(client, id, HELD_FOR).await.map_err(internal)?;
    Ok(client.clone().leased(id))
}

const HELD_FOR: Duration = Duration::from_secs(24 * 60 * 60);

pub(crate) async fn connect() -> RestResult<Containerd> {
    Containerd::connect().await.map_err(|error| {
        // A registry that cannot reach containerd has nowhere to put
        // anything, and saying "500" without the reason sends somebody
        // looking at their client.
        RestError::Internal(format!("containerd is not reachable: {error}"))
    })
}

pub(crate) fn internal(error: impl std::fmt::Display) -> RestError {
    RestError::Internal(error.to_string())
}

/// Where this request's bytes start: where its `Content-Range` says, and
/// otherwise after what containerd already holds.
///
/// Not zero. containerd reads a write at zero into a transaction that
/// has bytes as "start again" and truncates, so a second chunk sent
/// without a range would quietly throw the first away. And not the
/// request's length either: the final `PUT` of a layered upload carries
/// no body, no length and no range, and committing a 200 MB blob as zero
/// bytes fails — which the client answers by uploading the whole layer
/// again, and which reads as "it finished and then retried".
async fn starting_offset(
    client: &Containerd,
    session: &str,
    range_start: Option<i64>,
) -> RestResult<i64> {
    match range_start {
        Some(offset) => Ok(offset),
        None => content::written_so_far(client, session)
            .await
            .map_err(internal),
    }
}

fn content_range_start(request: &Request) -> Option<i64> {
    request
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split('-').next()?.trim().parse().ok())
}

fn query_digest(request: &Request) -> Option<String> {
    let query = request.uri().query()?;
    form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == "digest")
        .map(|(_, value)| value.into_owned())
}

/// `202` with where to send the next chunk.
///
/// The `Range` is inclusive of the last byte written, which is why it
/// is `end - 1` rather than `end` — a registry that gets this wrong
/// makes clients resend a byte on every chunk.
fn accepted(name: &str, session: &str, end: i64) -> Response {
    Response::builder()
        .status(StatusCode::ACCEPTED)
        .header(
            header::LOCATION,
            format!("/v2/{name}/blobs/uploads/{session}"),
        )
        .header("docker-upload-uuid", session)
        .header(header::RANGE, format!("0-{}", (end - 1).max(0)))
        .header(header::CONTENT_LENGTH, "0")
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

fn created(name: &str, digest: &str) -> Response {
    Response::builder()
        .status(StatusCode::CREATED)
        .header(header::LOCATION, format!("/v2/{name}/blobs/{digest}"))
        .header("docker-content-digest", digest)
        .header(header::CONTENT_LENGTH, "0")
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

fn missing() -> Response {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header("docker-distribution-api-version", "registry/2.0")
        .body(Body::empty())
        .expect("a constant response is well-formed")
}

fn digest_invalid(digest: &str, reason: &str) -> Response {
    let error = serde_json::json!({
        "code": "DIGEST_INVALID",
        "message": format!("the bytes uploaded do not hash to {digest}; push the blob again"),
        "detail": { "digest": digest, "reason": reason },
    });
    Response::builder()
        .status(StatusCode::BAD_REQUEST)
        .header(header::CONTENT_TYPE, "application/json")
        .header("docker-distribution-api-version", "registry/2.0")
        .body(Body::from(
            serde_json::json!({ "errors": [error] }).to_string(),
        ))
        .expect("a constant response is well-formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_chunk_goes_after_the_last_byte_written() {
        let response = accepted("demo/api", "session", 1024);
        let range = response
            .headers()
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok())
            .expect("a range");

        // Inclusive: 1024 bytes written are bytes 0 through 1023. Off
        // by one here makes every client resend a byte per chunk.
        assert_eq!(range, "0-1023");
    }

    /// An upload that has received nothing must not report a range
    /// ending at -1.
    #[test]
    fn an_empty_upload_reports_a_sane_range() {
        let response = accepted("demo/api", "session", 0);
        assert_eq!(
            response.headers().get(header::RANGE).unwrap(),
            "0-0",
            "not 0--1"
        );
    }

    #[test]
    fn the_digest_comes_out_of_the_query() {
        let request = Request::builder()
            .uri("/v2/demo/api/blobs/uploads/x?digest=sha256%3Aabc123")
            .body(Body::empty())
            .unwrap();
        assert_eq!(query_digest(&request).as_deref(), Some("sha256:abc123"));

        let bare = Request::builder()
            .uri("/v2/demo/api/blobs/uploads/x")
            .body(Body::empty())
            .unwrap();
        assert_eq!(query_digest(&bare), None);
    }

    #[test]
    fn a_content_range_says_where_the_chunk_belongs() {
        let request = Request::builder()
            .uri("/x")
            .header(header::CONTENT_RANGE, "4096-8191")
            .body(Body::empty())
            .unwrap();
        assert_eq!(content_range_start(&request), Some(4096));

        let without = Request::builder().uri("/x").body(Body::empty()).unwrap();
        assert_eq!(content_range_start(&without), None);
    }

    /// A layer sent whole in one `PUT` was read into memory under the
    /// manifest's limit, and BuildKit's first layer was refused with
    /// "length limit exceeded" before anything reached the store — from a
    /// GitHub Actions runner, while Docker, which `PATCH`es, pushed the
    /// same image. Under test the store is never reachable, so getting as
    /// far as asking for it is the claim.
    #[tokio::test]
    async fn a_layer_sent_whole_is_not_refused_for_its_size() {
        let whole = || {
            Request::builder()
                .uri("/x?digest=sha256%3Aabc")
                .body(Body::from(vec![0u8; 40 * 1024 * 1024]))
                .unwrap()
        };
        let refusals = [
            start("demo/api", whole()).await,
            finish("demo/api", "s", whole()).await,
        ];

        for refusal in refusals {
            let refusal = refusal.expect_err("there is no containerd under test");
            assert!(
                matches!(&refusal, RestError::Internal(why) if why.contains("containerd")),
                "refused before reaching the store: {refusal:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_body_arrives_in_pieces_of_a_megabyte_however_it_was_sent() {
        use futures_util::StreamExt;

        let frames = (0..2560).map(|_| Ok::<_, std::io::Error>(vec![7u8; 16 * 1024]));
        let pieces: Vec<Vec<u8>> = coalesced(futures_util::stream::iter(frames), IDLE)
            .map(|piece| piece.expect("nothing broke"))
            .collect()
            .await;

        assert_eq!(pieces.iter().map(Vec::len).sum::<usize>(), 40 * 1024 * 1024);
        assert!(
            pieces.iter().all(|piece| piece.len() == CHUNK),
            "{}",
            pieces.len()
        );
    }

    /// A body cut off half way used to read as a body that had ended, so
    /// the half was committed as the blob and the client was told about a
    /// digest rather than about its connection.
    #[tokio::test]
    async fn a_body_that_stops_arriving_ends_in_an_error() {
        use futures_util::StreamExt;

        let frames = vec![
            Ok(vec![1u8; 700 * 1024]),
            Ok(vec![2u8; 700 * 1024]),
            Err(std::io::Error::from(std::io::ErrorKind::ConnectionReset)),
            Ok(vec![3u8; 10]),
        ];
        let pieces: Vec<std::io::Result<Vec<u8>>> =
            coalesced(futures_util::stream::iter(frames), IDLE)
                .collect()
                .await;

        assert!(
            matches!(pieces.last(), Some(Err(_))),
            "the last thing said is that it broke: {} pieces",
            pieces.len()
        );
        assert_eq!(pieces.iter().filter(|piece| piece.is_err()).count(), 1);
    }

    /// A client that stopped sending without hanging up kept its
    /// connection and its half-written upload for as long as it stayed
    /// silent: ninety seconds on the VM, and no sign of an end.
    #[tokio::test]
    async fn a_client_that_goes_silent_is_given_up_on() {
        use futures_util::StreamExt;

        let silent = futures_util::stream::iter([Ok::<_, std::io::Error>(vec![1u8; 4096])])
            .chain(futures_util::stream::pending());
        let pieces: Vec<std::io::Result<Vec<u8>>> = tokio::time::timeout(
            Duration::from_secs(5),
            coalesced(silent, Duration::from_millis(50)).collect(),
        )
        .await
        .expect("the body ends while the client is still silent");

        let last = pieces.last().expect("something is said");
        assert!(
            matches!(last, Err(error) if error.kind() == std::io::ErrorKind::TimedOut),
            "{last:?}"
        );
    }

    /// A blob whose bytes do not hash to its digest answered 500 with
    /// containerd's sentence, which reads as the node failing. Measured on
    /// the VM: containerd says `FailedPrecondition`, "unexpected commit
    /// digest".
    #[tokio::test]
    async fn a_digest_that_does_not_match_is_refused_with_the_code_clients_print() {
        use http_body_util::BodyExt;

        let refusal = ClientError::Call {
            call: "Content.Write",
            source: tonic::Status::failed_precondition(
                "unexpected commit digest sha256:aaa, expected sha256:bbb: failed precondition",
            ),
        };
        let response = commit_refused(refusal, "sha256:bbb").expect("an answer, not a failure");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["errors"][0]["code"], "DIGEST_INVALID", "{json}");
        assert_eq!(json["errors"][0]["detail"]["digest"], "sha256:bbb");

        let unavailable = ClientError::Call {
            call: "Content.Write",
            source: tonic::Status::unavailable("containerd is restarting"),
        };
        assert!(
            matches!(
                commit_refused(unavailable, "sha256:bbb"),
                Err(RestError::Internal(_))
            ),
            "the node's own failures are still the node's"
        );
    }

    /// Each blob was read whole before its first byte went out: a 400 MB
    /// layer took the node from 27 to 427 MB on the VM, and node-0 had 481
    /// MB free when another node asked it for a 674 MB one (2026-11-14).
    #[tokio::test]
    async fn a_client_has_the_first_bytes_before_the_store_has_sent_the_last() {
        use futures_util::StreamExt;
        use http_body_util::BodyExt;

        let first = futures_util::stream::iter([Ok(b"head".to_vec())]);
        let response = serving(
            "sha256:abc",
            674_000_000,
            first.chain(futures_util::stream::pending()),
        );

        assert_eq!(
            response.headers().get(header::CONTENT_LENGTH).unwrap(),
            "674000000"
        );
        let mut body = response.into_body();
        let frame = tokio::time::timeout(std::time::Duration::from_secs(1), body.frame())
            .await
            .expect("the first piece arrives while the rest is still coming")
            .expect("a frame")
            .expect("not an error");
        assert_eq!(frame.into_data().unwrap(), "head");
    }

    /// The location a client follows for the next chunk has to name
    /// the same session, or every chunk starts a new upload.
    #[test]
    fn the_location_carries_the_session() {
        let response = accepted("demo/api", "abc-123", 10);
        assert_eq!(
            response.headers().get(header::LOCATION).unwrap(),
            "/v2/demo/api/blobs/uploads/abc-123"
        );
    }
}
