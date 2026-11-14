//! A database published from this node while its primary runs on another.
//!
//! The name resolves here — built from this node's domain, or chosen on
//! this node's page — and the node running the primary may have no public
//! address at all. So the port is opened here, the number this node chose,
//! as it is for a primary that runs here, and every connection is carried
//! over the overlay to the port that node binds for its copies.
//!
//! In this process rather than in the kernel. A DNAT rule would be the
//! portmap plugin's work done by hand, outside CNI, with nothing to take it
//! away when the process that wrote it is gone. A listener goes with the
//! process, and what it copies is TLS the server negotiates with the client
//! end to end.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};

use tokio::net::{TcpListener, TcpStream};

const CONNECT: std::time::Duration = std::time::Duration::from_secs(10);

pub struct Forwards {
    bind: IpAddr,
    running: tokio::sync::Mutex<BTreeMap<u16, Carried>>,
}

/// One open port: where it carries to, changeable without closing it.
///
/// A primary whose port changed keeps the listener. Closing it and binding
/// again is a race with the task that holds it, which lets go at its next
/// poll rather than when it is told to — and loses that race every time.
struct Carried {
    target: tokio::sync::watch::Sender<SocketAddr>,
    task: tokio::task::AbortHandle,
}

/// One per process, because two `Deployer` values in one process must not
/// both try to own a port — the reason `deploy_lock` is one too.
pub fn forwards() -> &'static Forwards {
    static FORWARDS: std::sync::OnceLock<Forwards> = std::sync::OnceLock::new();
    FORWARDS.get_or_init(|| Forwards::new(IpAddr::from([0, 0, 0, 0])))
}

impl Forwards {
    pub fn new(bind: IpAddr) -> Self {
        Self {
            bind,
            running: Default::default(),
        }
    }

    /// Listen on every port wanted, each carried to its target, and on no
    /// other. What could not be opened comes back with the reason.
    ///
    /// Convergent: a port already open stays open, so connections through
    /// it survive a pass that changes nothing, and one whose target moved
    /// sends the next connection to the new place.
    pub async fn converge(&self, wanted: &BTreeMap<u16, SocketAddr>) -> Vec<(u16, String)> {
        let mut running = self.running.lock().await;
        running.retain(|port, carried| match wanted.get(port) {
            Some(target) => {
                carried.target.send_if_modified(|now| {
                    let moved = now != target;
                    *now = *target;
                    moved
                });
                true
            }
            None => {
                carried.task.abort();
                false
            }
        });
        let mut failed = Vec::new();
        for (port, target) in wanted {
            if running.contains_key(port) {
                continue;
            }
            match TcpListener::bind((self.bind, *port)).await {
                Ok(listener) => {
                    let (sender, receiver) = tokio::sync::watch::channel(*target);
                    let task = tokio::spawn(carry(listener, receiver)).abort_handle();
                    running.insert(
                        *port,
                        Carried {
                            target: sender,
                            task,
                        },
                    );
                }
                Err(error) => failed.push((*port, error.to_string())),
            }
        }
        failed
    }
}

async fn carry(listener: TcpListener, target: tokio::sync::watch::Receiver<SocketAddr>) {
    loop {
        let mut client = match listener.accept().await {
            Ok((client, _)) => client,
            // Out of descriptors, most likely. Waiting is what gives one
            // back; accepting again at once is a loop that spins at a
            // hundred per cent and frees nothing — measured on the edge.
            Err(_) => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };
        let target = *target.borrow();
        tokio::spawn(async move {
            // A database speaks in small exchanges, each waiting on the
            // last, and Nagle holds every one of them back — the fault the
            // edge's listener had.
            let _ = client.set_nodelay(true);
            let Ok(Ok(mut server)) =
                tokio::time::timeout(CONNECT, TcpStream::connect(target)).await
            else {
                return;
            };
            let _ = server.set_nodelay(true);
            let _ = tokio::io::copy_bidirectional(&mut client, &mut server).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn echo() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let address = listener.local_addr().expect("address");
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let mut buffer = [0u8; 64];
                    while let Ok(read) = stream.read(&mut buffer).await {
                        if read == 0 || stream.write_all(&buffer[..read]).await.is_err() {
                            return;
                        }
                    }
                });
            }
        });
        address
    }

    fn a_free_port() -> u16 {
        std::net::TcpListener::bind("127.0.0.1:0")
            .expect("bind")
            .local_addr()
            .expect("address")
            .port()
    }

    async fn round_trip(port: u16) -> std::io::Result<Vec<u8>> {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).await?;
        stream.write_all(b"select 1").await?;
        let mut answer = vec![0u8; 8];
        stream.read_exact(&mut answer).await?;
        Ok(answer)
    }

    #[tokio::test]
    async fn a_published_port_reaches_the_primary_until_it_is_no_longer_wanted() {
        let forwards = Forwards::new(IpAddr::from([127, 0, 0, 1]));
        let port = a_free_port();
        let wanted = BTreeMap::from([(port, echo().await)]);

        assert!(forwards.converge(&wanted).await.is_empty());
        assert_eq!(round_trip(port).await.expect("carried"), b"select 1");

        let held = TcpStream::connect(("127.0.0.1", port))
            .await
            .expect("connected");
        assert!(
            forwards.converge(&wanted).await.is_empty(),
            "a pass that changes nothing opens nothing"
        );
        drop(held);

        forwards.converge(&BTreeMap::new()).await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            round_trip(port).await.is_err(),
            "an unpublished database is not reachable here"
        );
    }

    #[tokio::test]
    async fn a_primary_that_moved_its_port_is_reached_at_the_new_one() {
        let forwards = Forwards::new(IpAddr::from([127, 0, 0, 1]));
        let port = a_free_port();
        forwards
            .converge(&BTreeMap::from([(port, echo().await)]))
            .await;

        let gone = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let nothing_there = gone.local_addr().expect("address");
        drop(gone);
        forwards
            .converge(&BTreeMap::from([(port, nothing_there)]))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            round_trip(port).await.is_err(),
            "still carried to the old one"
        );

        forwards
            .converge(&BTreeMap::from([(port, echo().await)]))
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(round_trip(port).await.expect("carried"), b"select 1");
    }

    #[tokio::test]
    async fn a_port_somebody_else_holds_is_reported_rather_than_skipped() {
        let forwards = Forwards::new(IpAddr::from([127, 0, 0, 1]));
        let taken = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = taken.local_addr().expect("address").port();

        let failed = forwards
            .converge(&BTreeMap::from([(port, echo().await)]))
            .await;
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].0, port);
    }
}
