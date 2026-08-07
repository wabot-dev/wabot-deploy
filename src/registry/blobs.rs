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

use crate::runtime::client::Containerd;
use crate::runtime::content;

/// How much of one request's body is held in memory at a time.
///
/// A layer is hundreds of megabytes and the node has hundreds in
/// total, so the body is read in pieces and each is handed to
/// containerd before the next is read.
const CHUNK: usize = 4 * 1024 * 1024;

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
    let descriptor = containerd_client::types::Descriptor {
        digest: digest.to_string(),
        size: 0,
        ..Default::default()
    };

    match content::read(&client, &descriptor).await {
        Ok(bytes) => Ok(Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .header(header::CONTENT_LENGTH, bytes.len())
            .header("docker-content-digest", digest)
            .body(Body::from(bytes))
            .expect("a constant response is well-formed")),
        Err(_) => Ok(missing()),
    }
}

/// `POST /v2/<name>/blobs/uploads/` — begin.
///
/// Also handles the monolithic form, where the whole blob and its
/// digest arrive with the POST. Clients use it for small blobs, and a
/// registry that only implements the long form makes them do three
/// round trips for a two-kilobyte config.
pub async fn start(name: &str, request: Request) -> RestResult<Response> {
    let digest = query_digest(&request);
    let session = format!("wd-{}", wabot::prelude::password::generate(20));

    match digest {
        Some(digest) => {
            let body = read_body(request).await?;
            commit(&session, &digest, body).await?;
            Ok(created(name, &digest))
        }
        None => Ok(accepted(name, &session, 0)),
    }
}

/// `PATCH` — a chunk.
pub async fn patch(name: &str, session: &str, request: Request) -> RestResult<Response> {
    let offset = starting_offset(&request);
    let end = stream_into(session, offset, request).await?;
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

    // Whatever is left of the body belongs to this upload too: a
    // client is free to send everything in the PUT and never PATCH at
    // all.
    let offset = starting_offset(&request);
    let body = read_body(request).await?;

    let client = connect().await?;
    if body.is_empty() {
        // Nothing new — commit what the PATCHes already wrote.
        content::commit(
            &client,
            session,
            &digest,
            offset,
            Vec::new(),
            Default::default(),
        )
        .await
        .map_err(internal)?;
    } else {
        let total = offset + body.len() as i64;
        content::commit(&client, session, &digest, total, body, Default::default())
            .await
            .map_err(internal)?;
    }

    Ok(created(name, &digest))
}

/// Write a whole request body into a transaction, in pieces.
async fn stream_into(session: &str, offset: i64, request: Request) -> RestResult<i64> {
    let client = connect().await?;
    let mut body = request.into_body().into_data_stream();
    let mut at = offset;
    let mut buffered: Vec<u8> = Vec::new();

    use futures_util::StreamExt;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|error| RestError::Client {
            status: 400,
            message: format!("the upload stopped: {error}"),
        })?;
        buffered.extend_from_slice(&chunk);

        if buffered.len() >= CHUNK {
            at = content::write_chunk(&client, session, at, std::mem::take(&mut buffered))
                .await
                .map_err(internal)?;
        }
    }

    if !buffered.is_empty() {
        at = content::write_chunk(&client, session, at, buffered)
            .await
            .map_err(internal)?;
    }
    Ok(at)
}

/// Commit a blob that arrived whole.
async fn commit(session: &str, digest: &str, body: Vec<u8>) -> RestResult<()> {
    let client = connect().await?;
    content::commit(
        &client,
        session,
        digest,
        body.len() as i64,
        body,
        Default::default(),
    )
    .await
    .map_err(internal)
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

/// Where this request's bytes start.
///
/// From `Content-Range` when the client sent one. Clients that send
/// chunks in order without a range start at zero and stay in step,
/// which is the common case and what the header exists to disambiguate
/// when they do not.
fn starting_offset(request: &Request) -> i64 {
    request
        .headers()
        .get(header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split('-').next()?.trim().parse().ok())
        .unwrap_or(0)
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
        assert_eq!(starting_offset(&request), 4096);

        let without = Request::builder().uri("/x").body(Body::empty()).unwrap();
        assert_eq!(starting_offset(&without), 0);
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
