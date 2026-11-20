//! The door: one port on this node's overlay address, through which a node
//! holding a copy of a project reaches another copy of it here.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::{TcpListener, TcpStream};

use super::{read_line, Knock, DOOR_PORT, OPENED};
use crate::infra::deploy::Deployer;

const KNOCK_TIMEOUT: Duration = Duration::from_secs(5);
const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
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

/// One caller: its knock, whether it may, and then the copy.
pub(crate) async fn open(deployer: &Deployer, mut caller: TcpStream, peer: SocketAddr) {
    let knock = match tokio::time::timeout(KNOCK_TIMEOUT, read_line(&mut caller)).await {
        Ok(Ok(line)) => match serde_json::from_slice::<Knock>(&line) {
            Ok(knock) => knock,
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
    let target = match deployer.admit(peer.ip(), &knock).await {
        Ok(target) => target,
        Err(reason) => {
            tracing::warn!(%peer, project = %knock.project, service = %knock.service,
                slot = knock.slot, port = knock.port, %reason, "refused at the door");
            return;
        }
    };
    let mut copy = match tokio::time::timeout(DIAL_TIMEOUT, TcpStream::connect(target)).await {
        Ok(Ok(copy)) => copy,
        Ok(Err(error)) => {
            tracing::debug!(%target, %error, "the copy behind the door did not answer");
            return;
        }
        Err(_) => {
            tracing::debug!(%target, "the copy behind the door did not answer in time");
            return;
        }
    };
    if caller.write_all(OPENED).await.is_err() {
        return;
    }
    let _ = tokio::io::copy_bidirectional(&mut caller, &mut copy).await;
}
