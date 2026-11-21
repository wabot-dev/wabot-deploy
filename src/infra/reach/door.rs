//! The door: one port on this node's overlay address, through which a node
//! holding a copy of a project reaches another copy of it here, or leaves
//! for the internet when this node is one of the project's exits.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use super::{read_line, Asked, DOOR_PORT, OPENED};
use crate::infra::deploy::Deployer;

const KNOCK_TIMEOUT: Duration = Duration::from_secs(5);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// The internet is further away than a copy on this node.
pub(crate) const DIAL_OUT_TIMEOUT: Duration = Duration::from_secs(10);
const RETRY: Duration = Duration::from_secs(10);

/// Listen at the door for as long as the node runs. Bound to the overlay
/// address alone, and again whenever it cannot be: the tunnel comes up
/// after the node does, and a node joins a network long after it starts.
pub async fn run(deployer: Deployer) {
    loop {
        let Some(address) = deployer.overlay_ip().await else {
            tokio::time::sleep(RETRY).await;
            continue;
        };
        let listener = match bind(address) {
            Ok(listener) => listener,
            Err(error) => {
                tracing::debug!(%address, %error, "the door is not open yet");
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        tracing::info!(%address, port = DOOR_PORT, "door open for this node's projects");
        serve(&deployer, listener).await;
    }
}

/// Bound to the tunnel as well as to its address: Linux accepts a packet
/// for any of its addresses on any interface, so a neighbour on the
/// public network could otherwise send to the overlay address directly,
/// and the address a caller is judged by is only worth what WireGuard
/// vouches for.
fn bind(address: std::net::Ipv4Addr) -> std::io::Result<TcpListener> {
    let socket = tokio::net::TcpSocket::new_v4()?;
    socket.set_reuseaddr(true)?;
    #[cfg(target_os = "linux")]
    socket.bind_device(Some(crate::infra::network::tunnel::INTERFACE.as_bytes()))?;
    socket.bind((address, DOOR_PORT).into())?;
    socket.listen(128)
}

async fn serve(deployer: &Deployer, listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((caller, peer)) => {
                let deployer = deployer.clone();
                tokio::spawn(async move { open(&deployer, caller, peer).await });
            }
            Err(error) => {
                tracing::warn!(%error, "the door stopped accepting");
                return;
            }
        }
    }
}

/// One caller: its knock, whether it may, and then the copy or the
/// destination.
pub(crate) async fn open(deployer: &Deployer, mut caller: TcpStream, peer: SocketAddr) {
    let asked = match tokio::time::timeout(KNOCK_TIMEOUT, read_line(&mut caller)).await {
        Ok(Ok(line)) => match serde_json::from_slice::<Asked>(&line) {
            Ok(asked) => asked,
            Err(error) => {
                tracing::warn!(%peer, %error, "a knock that is not one");
                return;
            }
        },
        Ok(Err(error)) => {
            tracing::debug!(%peer, %error, "a caller left before knocking");
            return;
        }
        Err(_) => {
            tracing::debug!(%peer, "a caller that never knocked");
            return;
        }
    };
    let (admitted, patience) = match &asked {
        Asked::Reach(knock) => (deployer.admit(peer.ip(), knock).await, DIAL_TIMEOUT),
        Asked::Leave(leave) => (
            deployer.admit_leaving(peer.ip(), leave).await,
            DIAL_OUT_TIMEOUT,
        ),
    };
    let target = match admitted {
        Ok(target) => target,
        Err(reason) => {
            tracing::warn!(%peer, ?asked, %reason, "refused at the door");
            return;
        }
    };
    let mut beyond = match tokio::time::timeout(patience, TcpStream::connect(target)).await {
        Ok(Ok(beyond)) => beyond,
        Ok(Err(error)) => {
            tracing::debug!(%target, %error, "nothing answered behind the door");
            return;
        }
        Err(_) => {
            tracing::debug!(%target, "nothing answered behind the door in time");
            return;
        }
    };
    if caller.write_all(OPENED).await.is_err() {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut caller, &mut beyond).await;
}
