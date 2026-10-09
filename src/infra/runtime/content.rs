//! Reading blobs out of containerd's content store.
//!
//! Over the Content service rather than off the filesystem. The
//! architecture notes say the opposite for *serving* registry blobs —
//! framing megabytes through protobuf over a unix socket is real
//! overhead, and blobs are immutable and content-addressed, so reading
//! the file directly is safe there.
//!
//! Here the blobs are manifests and configs: a few kilobytes, read once
//! per deployment. Going through the API costs nothing measurable and
//! buys not caring where containerd keeps its files or which content
//! plugin is configured.

use std::collections::HashMap;

use containerd_client::services::v1::content_client::ContentClient;
use containerd_client::services::v1::{
    InfoRequest, ReadContentRequest, StatusRequest, WriteAction, WriteContentRequest,
};
use containerd_client::types::Descriptor;

use super::client::{ClientError, ClientResult, Containerd};

/// Everything at `descriptor.digest`.
///
/// The response is a stream of chunks; containerd decides the sizes and
/// a caller must not assume one message is the whole blob.
pub async fn read(client: &Containerd, descriptor: &Descriptor) -> ClientResult<Vec<u8>> {
    let mut stream = ContentClient::new(client.channel())
        .read(client.request(ReadContentRequest {
            digest: descriptor.digest.clone(),
            offset: 0,
            size: 0,
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Content.Read",
            source,
        })?
        .into_inner();

    let mut bytes = Vec::with_capacity(descriptor.size.max(0) as usize);
    while let Some(chunk) = stream.message().await.map_err(|source| ClientError::Call {
        call: "Content.Read",
        source,
    })? {
        bytes.extend_from_slice(&chunk.data);
    }

    // The descriptor's size is part of the contract, and a short read
    // here would otherwise surface as a JSON parse error that says
    // nothing about the transfer having been truncated.
    if descriptor.size > 0 && bytes.len() as i64 != descriptor.size {
        return Err(ClientError::Other(format!(
            "{} is {} bytes, expected {}",
            descriptor.digest,
            bytes.len(),
            descriptor.size
        )));
    }

    Ok(bytes)
}

/// Is this blob already here?
///
/// What a registry answers `HEAD /v2/<name>/blobs/<digest>` with, and
/// what lets a `docker push` skip a layer the node already has —
/// which, on a node that shares one content store between the registry
/// and the runtime, is most of them.
pub async fn exists(client: &Containerd, digest: &str) -> ClientResult<Option<i64>> {
    let response = ContentClient::new(client.channel())
        .info(client.request(InfoRequest {
            digest: digest.to_string(),
        }))
        .await;

    match response {
        Ok(response) => Ok(response.into_inner().info.map(|info| info.size)),
        Err(status) if status.code() == tonic::Code::NotFound => Ok(None),
        Err(source) => Err(ClientError::Call {
            call: "Content.Info",
            source,
        }),
    }
}

/// How far an open transaction has got.
///
/// The only honest source for this. A registry client finishing an
/// upload sends an empty `PUT` with no size and no range — everything
/// it had, it already sent — so the number has to come from the side
/// that stored the bytes. Guessing it from a request header means
/// committing a 200 MB layer as though it were zero bytes long, which
/// containerd refuses and the client answers by uploading the whole
/// layer again. That is exactly what "every blob finishes and then
/// retries" looks like from the outside.
pub async fn written_so_far(client: &Containerd, ref_: &str) -> ClientResult<i64> {
    let response = ContentClient::new(client.channel())
        .status(client.request(StatusRequest {
            r#ref: ref_.to_string(),
        }))
        .await;

    match response {
        Ok(response) => Ok(response.into_inner().status.map(|s| s.offset).unwrap_or(0)),
        // No transaction under that ref: nothing has been written, and
        // an upload that sends everything in one `PUT` looks like this.
        Err(status) if status.code() == tonic::Code::NotFound => Ok(0),
        Err(source) => Err(ClientError::Call {
            call: "Content.Status",
            source,
        }),
    }
}

/// Write a whole sequence of chunks into one transaction.
///
/// `ref_` names the transaction and `offset` says where the first
/// chunk goes. containerd's write is keyed on the ref rather than on
/// the connection, so a later HTTP request can open another
/// transaction against the same ref and carry on — which is what makes
/// a resumable upload possible without this module holding any state
/// between requests.
///
/// **One stream for the whole body.** An earlier version opened one
/// per chunk: fifty stream set-ups for a 200 MB layer, on a node with
/// one core also decrypting four parallel uploads. The cost showed up
/// as TLS handshake timeouts on the connections waiting behind them.
pub async fn write_chunks<S>(
    client: &Containerd,
    ref_: &str,
    offset: i64,
    chunks: S,
) -> ClientResult<i64>
where
    S: futures_util::Stream<Item = Vec<u8>> + Send + 'static,
{
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::Arc;

    // Where the write got to, shared with the stream that is producing
    // the messages — the caller needs it back, and the stream is what
    // knows.
    let at = Arc::new(AtomicI64::new(offset));
    let counter = at.clone();
    let ref_owned = ref_.to_string();

    let requests = async_stream::stream! {
        futures_util::pin_mut!(chunks);
        use futures_util::StreamExt;

        while let Some(data) = chunks.next().await {
            let size = data.len() as i64;
            let start = counter.fetch_add(size, Ordering::SeqCst);
            yield WriteContentRequest {
                action: WriteAction::Write as i32,
                r#ref: ref_owned.clone(),
                offset: start,
                data,
                ..Default::default()
            };
        }
    };

    let mut responses = ContentClient::new(client.channel())
        .write(client.request(requests))
        .await
        .map_err(|source| ClientError::Call {
            call: "Content.Write",
            source,
        })?
        .into_inner();

    while responses
        .message()
        .await
        .map_err(|source| ClientError::Call {
            call: "Content.Write",
            source,
        })?
        .is_some()
    {}

    Ok(at.load(Ordering::SeqCst))
}

/// Close a transaction, storing what was written under `digest`.
///
/// containerd verifies the content against the digest and refuses if
/// they disagree, which is exactly the check a registry owes its
/// clients — and doing it here means it happens once, in the place
/// that stores the bytes.
///
/// `labels` is how the blob survives garbage collection: containerd's
/// GC starts from images and follows `containerd.io/gc.ref.content.*`
/// labels, so a manifest that does not point at its layers is a
/// manifest whose layers get collected out from under it.
pub async fn commit(
    client: &Containerd,
    ref_: &str,
    digest: &str,
    total: i64,
    data: Vec<u8>,
    labels: HashMap<String, String>,
) -> ClientResult<()> {
    let request = WriteContentRequest {
        action: WriteAction::Commit as i32,
        r#ref: ref_.to_string(),
        total,
        expected: digest.to_string(),
        offset: total - data.len() as i64,
        data,
        labels,
    };

    let result = ContentClient::new(client.channel())
        .write(client.request(tokio_stream::iter(vec![request])))
        .await;

    let mut stream = match result {
        Ok(response) => response.into_inner(),
        // Somebody pushed this blob already, or it arrived with an
        // image. Content is addressed by its digest, so the bytes are
        // the same bytes — there is nothing to do and nothing to
        // report.
        Err(status) if status.code() == tonic::Code::AlreadyExists => return Ok(()),
        Err(source) => {
            return Err(ClientError::Call {
                call: "Content.Write",
                source,
            })
        }
    };

    while stream
        .message()
        .await
        .map_err(|source| ClientError::Call {
            call: "Content.Write",
            source,
        })?
        .is_some()
    {}

    Ok(())
}

/// The labels that keep a manifest's dependencies alive.
///
/// One per referenced blob — the config and every layer. The key's
/// suffix only has to be unique within the label set; containerd reads
/// the *values*.
pub fn gc_labels(digests: &[String]) -> HashMap<String, String> {
    digests
        .iter()
        .enumerate()
        .map(|(index, digest)| {
            (
                format!("containerd.io/gc.ref.content.{index}"),
                digest.clone(),
            )
        })
        .collect()
}
