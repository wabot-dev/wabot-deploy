//! Where a container's connection to a stand-in arrives, and leaves again
//! for the door of a node running the copy.

use std::net::{SocketAddr, SocketAddrV4};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use super::{Knock, NAMES_PORT, OPENED};
use crate::infra::deploy::Deployer;

const DIAL_TIMEOUT: Duration = Duration::from_secs(3);
const RETRY: Duration = Duration::from_secs(10);

/// Where one connection may go: a door, and what to say there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub door: SocketAddr,
    pub knock: Knock,
}

/// Listen for every container of every project. On all addresses, because
/// the `REDIRECT` delivers to whichever bridge's gateway the connection
/// came in on; [`open`] refuses anything that did not come through one.
pub async fn run(deployer: Deployer) {
    loop {
        let listener = match TcpListener::bind(("0.0.0.0", NAMES_PORT)).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(port = NAMES_PORT, %error, "names on other nodes cannot be reached");
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        loop {
            match listener.accept().await {
                Ok((caller, SocketAddr::V4(peer))) => {
                    let deployer = deployer.clone();
                    tokio::spawn(async move { open(&deployer, caller, peer).await });
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "names stopped accepting");
                    break;
                }
            }
        }
    }
}

async fn open(deployer: &Deployer, mut caller: TcpStream, peer: SocketAddrV4) {
    let asked = match original_destination(&caller) {
        Ok(asked) => asked,
        Err(error) => {
            tracing::debug!(%peer, %error, "a connection that was not redirected");
            return;
        }
    };
    let Ok(SocketAddr::V4(arrived)) = caller.local_addr() else {
        return;
    };
    let routes = match deployer.route(peer, *arrived.ip(), asked).await {
        Ok(routes) => routes,
        Err(reason) => {
            tracing::debug!(%peer, %asked, %reason, "nowhere to send a stand-in's connection");
            return;
        }
    };
    for route in routes {
        if let Some(mut door) = knock(&route).await {
            let _ = tokio::io::copy_bidirectional(&mut caller, &mut door).await;
            return;
        }
        tracing::debug!(door = %route.door, service = %route.knock.service, "a door did not open");
    }
}

/// Knock at one door, and hand back the connection once it opened.
async fn knock(route: &Route) -> Option<TcpStream> {
    let attempt = async {
        let mut door = TcpStream::connect(route.door).await.ok()?;
        door.write_all(&route.knock.line()).await.ok()?;
        let mut answer = [0u8; OPENED.len()];
        door.read_exact(&mut answer).await.ok()?;
        (answer == OPENED).then_some(door)
    };
    tokio::time::timeout(DIAL_TIMEOUT, attempt)
        .await
        .ok()
        .flatten()
}

/// The address the container asked for, before the `REDIRECT` changed it.
#[cfg(target_os = "linux")]
fn original_destination(stream: &TcpStream) -> std::io::Result<SocketAddrV4> {
    use std::os::fd::AsRawFd;
    // SAFETY: `getsockopt` writes at most `length` bytes into `address`,
    // which is a `sockaddr_in` of exactly that size, and the descriptor is
    // the stream's own for the duration of the call.
    let mut address: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    let mut length = std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_IP,
            libc::SO_ORIGINAL_DST,
            &mut address as *mut libc::sockaddr_in as *mut libc::c_void,
            &mut length,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(SocketAddrV4::new(
        u32::from_be(address.sin_addr.s_addr).into(),
        u16::from_be(address.sin_port),
    ))
}

#[cfg(not(target_os = "linux"))]
fn original_destination(_stream: &TcpStream) -> std::io::Result<SocketAddrV4> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "a redirected connection is a Linux kernel's to describe",
    ))
}
