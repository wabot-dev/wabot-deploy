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
    InfoRequest, ReadContentRequest, WriteAction, WriteContentRequest,
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

/// Write bytes into an open transaction, without committing it.
///
/// `ref_` names the transaction, `offset` says where these bytes go.
/// Both come straight from the upload the registry is serving, which
/// is why this is resumable across separate HTTP requests: containerd's
/// write is keyed on the ref, not on the connection that opened it.
/// The alternative — holding a gRPC stream open between requests — is
/// a stream per upload in a map, and a leak whenever a client walks
/// away mid-push.
pub async fn write_chunk(
    client: &Containerd,
    ref_: &str,
    offset: i64,
    data: Vec<u8>,
) -> ClientResult<i64> {
    let written = data.len() as i64;
    let request = WriteContentRequest {
        action: WriteAction::Write as i32,
        r#ref: ref_.to_string(),
        offset,
        data,
        ..Default::default()
    };

    // The stream carries one message and ends. containerd keeps the
    // transaction; the next chunk opens its own stream at the next
    // offset.
    let mut stream = ContentClient::new(client.channel())
        .write(client.request(tokio_stream::iter(vec![request])))
        .await
        .map_err(|source| ClientError::Call {
            call: "Content.Write",
            source,
        })?
        .into_inner();

    while stream
        .message()
        .await
        .map_err(|source| ClientError::Call {
            call: "Content.Write",
            source,
        })?
        .is_some()
    {}

    Ok(offset + written)
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
