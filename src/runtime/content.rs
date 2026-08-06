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

use containerd_client::services::v1::content_client::ContentClient;
use containerd_client::services::v1::ReadContentRequest;
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
