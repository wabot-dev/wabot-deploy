//! Reading blobs out of containerd's content store.
//!
//! Over the Content service rather than off the filesystem: it buys not
//! caring where containerd keeps its files or which content plugin is
//! configured.

use std::collections::HashMap;

use containerd_client::services::v1::content_client::ContentClient;
use containerd_client::services::v1::{
    AbortRequest, Info, InfoRequest, ReadContentRequest, StatusRequest, UpdateRequest, WriteAction,
    WriteContentRequest,
};
use containerd_client::types::Descriptor;

use super::client::{ClientError, ClientResult, Containerd};

/// Everything at `descriptor.digest`, in memory — for manifests and configs.
/// A layer goes through [`stream`].
pub async fn read(client: &Containerd, descriptor: &Descriptor) -> ClientResult<Vec<u8>> {
    use futures_util::StreamExt;

    let mut chunks = stream(client, &descriptor.digest).await?;
    let mut bytes = Vec::with_capacity(descriptor.size.max(0) as usize);
    while let Some(chunk) = chunks.next().await {
        bytes.extend_from_slice(&chunk?);
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

/// The blob at `digest`, in the pieces containerd sends it in.
pub async fn stream(
    client: &Containerd,
    digest: &str,
) -> ClientResult<impl futures_util::Stream<Item = ClientResult<Vec<u8>>> + Send + Unpin + 'static>
{
    use futures_util::StreamExt;

    let chunks = ContentClient::new(client.channel())
        .read(client.request(ReadContentRequest {
            digest: digest.to_string(),
            offset: 0,
            size: 0,
        }))
        .await
        .map_err(|source| ClientError::Call {
            call: "Content.Read",
            source,
        })?
        .into_inner();

    Ok(chunks.map(|chunk| {
        chunk
            .map(|chunk| chunk.data)
            .map_err(|source| ClientError::Call {
                call: "Content.Read",
                source,
            })
    }))
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
///
/// A chunk that is an error ends the write there and is the answer:
/// what arrived before it stays in the transaction, where a resuming
/// client finds it.
pub async fn write_chunks<S>(
    client: &Containerd,
    ref_: &str,
    offset: i64,
    chunks: S,
) -> ClientResult<i64>
where
    S: futures_util::Stream<Item = std::io::Result<Vec<u8>>> + Send + 'static,
{
    write(client, ref_, offset, chunks, None).await
}

/// [`write_chunks`], then the commit under `digest`, on the same stream.
///
/// Opening a second stream to commit makes containerd resume the
/// transaction, and resuming reads back and hashes every byte already
/// written: a whole layer, again, on a one-core node. containerd's own
/// client sends the commit last on the stream that wrote.
///
/// Nothing is committed when a chunk is an error.
pub async fn write_and_commit<S>(
    client: &Containerd,
    ref_: &str,
    offset: i64,
    chunks: S,
    digest: &str,
    labels: HashMap<String, String>,
) -> ClientResult<i64>
where
    S: futures_util::Stream<Item = std::io::Result<Vec<u8>>> + Send + 'static,
{
    let seal = Seal {
        expected: digest.to_string(),
        labels,
    };
    write(client, ref_, offset, chunks, Some(seal)).await
}

struct Seal {
    expected: String,
    labels: HashMap<String, String>,
}

async fn write<S>(
    client: &Containerd,
    ref_: &str,
    offset: i64,
    chunks: S,
    seal: Option<Seal>,
) -> ClientResult<i64>
where
    S: futures_util::Stream<Item = std::io::Result<Vec<u8>>> + Send + 'static,
{
    use futures_util::StreamExt;
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::sync::{Arc, Mutex};

    // A write's first message is what names its transaction, so a stream
    // with no message names nothing and containerd refuses it.
    let mut chunks = Box::pin(chunks.peekable());
    let sealing = seal.is_some();
    if !sealing && chunks.as_mut().peek().await.is_none() {
        return written_so_far(client, ref_).await;
    }

    let at = Arc::new(AtomicI64::new(offset));
    let counter = at.clone();
    let broken: Arc<Mutex<Option<std::io::Error>>> = Arc::new(Mutex::new(None));
    let breaks = broken.clone();
    let ref_owned = ref_.to_string();

    let requests = async_stream::stream! {
        while let Some(next) = chunks.next().await {
            let data = match next {
                Ok(data) => data,
                Err(error) => {
                    if let Ok(mut slot) = breaks.lock() {
                        *slot = Some(error);
                    }
                    return;
                }
            };
            let start = counter.fetch_add(data.len() as i64, Ordering::SeqCst);
            yield WriteContentRequest {
                action: WriteAction::Write as i32,
                r#ref: ref_owned.clone(),
                offset: start,
                data,
                ..Default::default()
            };
        }
        if let Some(Seal { expected, labels }) = seal {
            let total = counter.load(Ordering::SeqCst);
            yield WriteContentRequest {
                action: WriteAction::Commit as i32,
                r#ref: ref_owned,
                total,
                expected,
                offset: total,
                data: Vec::new(),
                labels,
            };
        }
    };

    let outcome = match ContentClient::new(client.channel())
        .write(client.request(requests))
        .await
    {
        Ok(response) => {
            let mut responses = response.into_inner();
            loop {
                match responses.message().await {
                    Ok(Some(_)) => {}
                    Ok(None) => break Ok(()),
                    Err(status) => break Err(status),
                }
            }
        }
        Err(status) => Err(status),
    };

    let interrupted = broken.lock().ok().and_then(|mut slot| slot.take());
    if let Some(error) = interrupted {
        return Err(ClientError::Interrupted(error));
    }

    match outcome {
        Ok(()) => Ok(at.load(Ordering::SeqCst)),
        // Somebody pushed this blob already, or it arrived with an image.
        // Content is addressed by its digest, so the bytes are the same
        // bytes. Aborted because containerd refuses an expected digest it
        // has before it looks at the transaction, which would otherwise
        // keep its copy under the lease for a day.
        Err(status) if sealing && status.code() == tonic::Code::AlreadyExists => {
            if let Err(error) = abort(client, ref_).await {
                tracing::warn!(transaction = %ref_, %error, "a blob already here left its upload behind");
            }
            Ok(at.load(Ordering::SeqCst))
        }
        Err(source) => Err(ClientError::Call {
            call: "Content.Write",
            source,
        }),
    }
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
    mut data: Vec<u8>,
    labels: HashMap<String, String>,
) -> ClientResult<()> {
    // From where the transaction already stands. A ref is keyed on what
    // it carries — a digest, an upload session — so bytes already in it
    // are these bytes.
    let start = total - data.len() as i64;
    let current = written_so_far(client, ref_).await?;
    if current > start && current <= total {
        data.drain(..(current - start) as usize);
    }
    let from = total - data.len() as i64;

    // **In pieces, and in one stream, whatever the caller had.**
    // containerd takes no gRPC message over 16 MB, so a blob handed over
    // whole — a layer restored from a backup — was refused with "received
    // message larger than max" from the first layer of any real image.
    // Found on a VM node restoring a backup: the images it had kept for
    // being the only copy did not come back. Writing the head on one
    // stream and committing on a second was then refused at the commit's
    // offset, so the writes and the commit share a stream.
    //
    // Cut as the stream asks for them, so a restored layer is in memory
    // once and not twice.
    let pieces = async_stream::stream! {
        for piece in data.chunks(PIECE) {
            yield Ok(piece.to_vec());
        }
    };
    write_and_commit(client, ref_, from, pieces, digest, labels)
        .await
        .map(|_| ())
}

/// Throw away a transaction, so the next attempt starts from nothing.
pub async fn abort(client: &Containerd, ref_: &str) -> ClientResult<()> {
    match ContentClient::new(client.channel())
        .abort(client.request(AbortRequest {
            r#ref: ref_.to_string(),
        }))
        .await
    {
        Ok(_) => Ok(()),
        Err(status) if status.code() == tonic::Code::NotFound => Ok(()),
        Err(source) => Err(ClientError::Call {
            call: "Content.Abort",
            source,
        }),
    }
}

/// Set one label on a blob, leaving its others as they are.
pub async fn label(client: &Containerd, digest: &str, key: &str, value: &str) -> ClientResult<()> {
    ContentClient::new(client.channel())
        .update(client.request(UpdateRequest {
            info: Some(Info {
                digest: digest.to_string(),
                labels: HashMap::from([(key.to_string(), value.to_string())]),
                ..Default::default()
            }),
            update_mask: Some(prost_types::FieldMask {
                paths: vec![format!("labels.{key}")],
            }),
        }))
        .await
        .map(|_| ())
        .map_err(|source| ClientError::Call {
            call: "Content.Update",
            source,
        })
}

pub async fn has_label(client: &Containerd, digest: &str, key: &str) -> ClientResult<bool> {
    let response = ContentClient::new(client.channel())
        .info(client.request(InfoRequest {
            digest: digest.to_string(),
        }))
        .await;

    match response {
        Ok(response) => Ok(response
            .into_inner()
            .info
            .is_some_and(|info| info.labels.contains_key(key))),
        Err(status) if status.code() == tonic::Code::NotFound => Ok(false),
        Err(source) => Err(ClientError::Call {
            call: "Content.Info",
            source,
        }),
    }
}

/// A label named in the mask and absent from the map is deleted.
pub async fn unlabel(client: &Containerd, digest: &str, key: &str) -> ClientResult<()> {
    ContentClient::new(client.channel())
        .update(client.request(UpdateRequest {
            info: Some(Info {
                digest: digest.to_string(),
                ..Default::default()
            }),
            update_mask: Some(prost_types::FieldMask {
                paths: vec![format!("labels.{key}")],
            }),
        }))
        .await
        .map(|_| ())
        .map_err(|source| ClientError::Call {
            call: "Content.Update",
            source,
        })
}

/// The most one message carries, well under containerd's 16 MB ceiling.
const PIECE: usize = 1024 * 1024;

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
