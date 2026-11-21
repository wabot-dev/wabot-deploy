//! Where a container's connection to the internet arrives, on a node that
//! is not one of its project's exits, and leaves again for the door of one
//! that is. Starting at a different exit each time, and failing when none
//! opens: leaving through this node instead would be leaving from an
//! address nobody chose.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::{TcpListener, TcpStream};

use super::EXIT_PORT;
use crate::infra::deploy::Deployer;
use crate::infra::reach::door::DIAL_OUT_TIMEOUT;
use crate::infra::reach::names::{knock, original_destination};

const RETRY: Duration = Duration::from_secs(10);

pub async fn run(deployer: Deployer) {
    loop {
        let listener = match TcpListener::bind(("0.0.0.0", EXIT_PORT)).await {
            Ok(listener) => listener,
            Err(error) => {
                tracing::warn!(port = EXIT_PORT, %error, "projects cannot leave through their exits");
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        loop {
            match listener.accept().await {
                Ok((caller, SocketAddr::V4(peer))) => {
                    let deployer = deployer.clone();
                    tokio::spawn(async move { carry(&deployer, caller, peer).await });
                }
                Ok(_) => {}
                Err(error) => {
                    tracing::warn!(%error, "the exit listener stopped accepting");
                    break;
                }
            }
        }
    }
}

async fn carry(deployer: &Deployer, mut caller: TcpStream, peer: std::net::SocketAddrV4) {
    let Ok(to) = original_destination(&caller) else {
        return;
    };
    let Ok(SocketAddr::V4(arrived)) = caller.local_addr() else {
        return;
    };
    let (leave, doors) = match deployer.exits_for(peer, *arrived.ip(), to).await {
        Ok(found) => found,
        Err(reason) => {
            tracing::debug!(%peer, %to, %reason, "no exit for a connection");
            return;
        }
    };
    let line = leave.line();
    for door in &doors {
        if let Some(mut exit) = knock(*door, &line, DIAL_OUT_TIMEOUT + Duration::from_secs(2)).await
        {
            let _ = tokio::io::copy_bidirectional(&mut caller, &mut exit).await;
            return;
        }
        tracing::debug!(%door, %to, "an exit did not open");
    }
    tracing::warn!(project = %leave.project, %to, exits = doors.len(), "no exit opened, so the connection was refused");
}
