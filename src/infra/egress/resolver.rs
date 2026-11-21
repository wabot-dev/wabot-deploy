//! Port 53 of every closed project, redirected here by `nat`.
//!
//! Bound on every address and answering only a project's containers, by
//! the bridge the source address is on: a resolver the internet could ask
//! would be somebody else's amplifier.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use hickory_server::proto::op::{Message, ResponseCode};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};

use super::{addresses_in, answer_with, decide, Decision, RESOLVER_PORT};
use crate::domain::values::destinations::normalised;
use crate::infra::deploy::Deployer;

const UPSTREAM_PATIENCE: Duration = Duration::from_secs(2);
const IDLE: Duration = Duration::from_secs(10);
const LARGEST: usize = 65_535;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Over {
    Udp,
    Tcp,
}

pub async fn run(deployer: Deployer) {
    let address = SocketAddr::from((Ipv4Addr::UNSPECIFIED, RESOLVER_PORT));
    loop {
        match (
            UdpSocket::bind(address).await,
            TcpListener::bind(address).await,
        ) {
            (Ok(udp), Ok(tcp)) => {
                tokio::select! {
                    error = serve_udp(&deployer, Arc::new(udp)) => {
                        tracing::warn!(%error, "the projects' resolver stopped answering over UDP");
                    }
                    error = serve_tcp(&deployer, tcp) => {
                        tracing::warn!(%error, "the projects' resolver stopped answering over TCP");
                    }
                }
            }
            (udp, tcp) => {
                let error = udp.err().or(tcp.err()).map(|error| error.to_string());
                tracing::warn!(
                    ?error,
                    port = RESOLVER_PORT,
                    "could not answer the projects' DNS"
                );
            }
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

async fn serve_udp(deployer: &Deployer, socket: Arc<UdpSocket>) -> std::io::Error {
    let mut buffer = vec![0u8; LARGEST];
    loop {
        let (length, peer) = match socket.recv_from(&mut buffer).await {
            Ok(received) => received,
            Err(error) => return error,
        };
        let query = buffer[..length].to_vec();
        let deployer = deployer.clone();
        let socket = socket.clone();
        tokio::spawn(async move {
            let upstreams = deployer.upstreams();
            if let Some(reply) = answer(&deployer, &upstreams, &query, peer.ip(), Over::Udp).await {
                let _ = socket.send_to(&reply, peer).await;
            }
        });
    }
}

async fn serve_tcp(deployer: &Deployer, listener: TcpListener) -> std::io::Error {
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(error) => return error,
        };
        let deployer = deployer.clone();
        tokio::spawn(async move {
            let _ = converse(&deployer, stream, peer.ip()).await;
        });
    }
}

async fn converse(deployer: &Deployer, mut stream: TcpStream, peer: IpAddr) -> std::io::Result<()> {
    loop {
        let Ok(query) = tokio::time::timeout(IDLE, read_framed(&mut stream)).await else {
            return Ok(());
        };
        let upstreams = deployer.upstreams();
        let Some(reply) = answer(deployer, &upstreams, &query?, peer, Over::Tcp).await else {
            return Ok(());
        };
        write_framed(&mut stream, &reply).await?;
    }
}

/// What to send back to `source`, or nothing for a source that is no
/// project's container or a query that does not read.
pub(crate) async fn answer(
    deployer: &Deployer,
    upstreams: &[SocketAddr],
    query: &[u8],
    source: IpAddr,
    over: Over,
) -> Option<Vec<u8>> {
    let IpAddr::V4(source) = source.to_canonical() else {
        return None;
    };
    let (project_id, destinations) = deployer.egress_from(source).await?;
    let message = Message::from_vec(query).ok()?;
    let name = normalised(&message.queries.first()?.name.to_ascii());
    match decide(&destinations, &name) {
        Decision::Refuse => {
            deployer.note_refused(&project_id, &name).await;
            answer_with(&message, ResponseCode::NXDomain).to_vec().ok()
        }
        Decision::Ask { opens } => {
            let Some(reply) = ask_upstream(upstreams, query, over).await else {
                return answer_with(&message, ResponseCode::ServFail).to_vec().ok();
            };
            if !opens.is_empty() {
                if let Ok(parsed) = Message::from_vec(&reply) {
                    let (addresses, cached) = addresses_in(&parsed);
                    deployer
                        .open_learned(&project_id, &name, &addresses, &opens, cached)
                        .await;
                }
            }
            Some(reply)
        }
    }
}

async fn ask_upstream(upstreams: &[SocketAddr], query: &[u8], over: Over) -> Option<Vec<u8>> {
    for upstream in upstreams {
        let asked = match over {
            Over::Udp => ask_udp(*upstream, query).await,
            Over::Tcp => ask_tcp(*upstream, query).await,
        };
        if let Some(reply) = asked.filter(|reply| reply.get(..2) == query.get(..2)) {
            return Some(reply);
        }
    }
    None
}

async fn ask_udp(upstream: SocketAddr, query: &[u8]) -> Option<Vec<u8>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    socket.connect(upstream).await.ok()?;
    socket.send(query).await.ok()?;
    let mut buffer = vec![0u8; LARGEST];
    let length = tokio::time::timeout(UPSTREAM_PATIENCE, socket.recv(&mut buffer))
        .await
        .ok()?
        .ok()?;
    buffer.truncate(length);
    Some(buffer)
}

async fn ask_tcp(upstream: SocketAddr, query: &[u8]) -> Option<Vec<u8>> {
    let exchange = async {
        let mut stream = TcpStream::connect(upstream).await?;
        write_framed(&mut stream, query).await?;
        read_framed(&mut stream).await
    };
    tokio::time::timeout(UPSTREAM_PATIENCE * 2, exchange)
        .await
        .ok()?
        .ok()
}

async fn read_framed(stream: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let length = stream.read_u16().await?;
    let mut message = vec![0u8; usize::from(length)];
    stream.read_exact(&mut message).await?;
    Ok(message)
}

async fn write_framed(stream: &mut TcpStream, message: &[u8]) -> std::io::Result<()> {
    let length = u16::try_from(message.len())
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "too long"))?;
    let mut framed = Vec::with_capacity(message.len() + 2);
    framed.extend_from_slice(&length.to_be_bytes());
    framed.extend_from_slice(message);
    stream.write_all(&framed).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::destinations::Destinations;
    use crate::infra::repositories::{egress, projects};
    use hickory_server::proto::op::Query;
    use hickory_server::proto::rr::rdata::A;
    use hickory_server::proto::rr::{Name, RData, Record, RecordType};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// An upstream that answers every A question with 198.51.100.7, and
    /// counts what it was asked.
    async fn upstream() -> (SocketAddr, Arc<AtomicUsize>) {
        let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind");
        let address = socket.local_addr().expect("address");
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = asked.clone();
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 4096];
            while let Ok((length, peer)) = socket.recv_from(&mut buffer).await {
                counted.fetch_add(1, Ordering::SeqCst);
                let query = Message::from_vec(&buffer[..length]).expect("a query");
                let mut reply = answer_with(&query, ResponseCode::NoError);
                reply.answers = vec![Record::from_rdata(
                    query.queries[0].name.clone(),
                    60,
                    RData::A(A("198.51.100.7".parse().expect("an address"))),
                )];
                let _ = socket.send_to(&reply.to_vec().expect("encode"), peer).await;
            }
        });
        (address, asked)
    }

    fn question(name: &str) -> Vec<u8> {
        let mut query = Message::query();
        query.metadata.recursion_desired = true;
        query.queries = vec![Query::query(
            Name::from_ascii(name).expect("a name"),
            RecordType::A,
        )];
        query.to_vec().expect("encode")
    }

    #[tokio::test]
    async fn a_listed_name_is_answered_and_opened_and_an_unlisted_one_is_never_asked() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");
        let index = projects::ensure_network_index(&database, &project.id)
            .await
            .expect("a network");
        egress::set(
            &database,
            &project.id,
            &Destinations::parse("api.example.com:443").expect("a list"),
        )
        .await
        .expect("set");
        let deployer = Deployer::new(
            Arc::new(database.clone()),
            &crate::infra::config::Config::default(),
        );
        let (upstream, asked) = upstream().await;
        let container = IpAddr::from([10, 42, index, 5]);
        let ask = |name: &str, from: IpAddr| {
            let query = question(name);
            let deployer = deployer.clone();
            async move { answer(&deployer, &[upstream], &query, from, Over::Udp).await }
        };

        let reply = ask("API.example.com.", container).await.expect("answered");
        let reply = Message::from_vec(&reply).expect("a reply");
        assert_eq!(reply.answers.len(), 1);
        let opened = egress::learned(&database, &project.id, crate::infra::clock::now_ms())
            .await
            .expect("read");
        assert_eq!(
            opened,
            vec![("198.51.100.7".parse().expect("an address"), Some(443))]
        );
        let still_open = crate::infra::clock::now_ms() + 59 * 60 * 1000;
        assert!(
            !egress::learned(&database, &project.id, still_open)
                .await
                .expect("read")
                .is_empty(),
            "a 60-second answer stays open for the hour"
        );

        let refused = ask("leak.attacker.example.", container)
            .await
            .expect("answered");
        let refused = Message::from_vec(&refused).expect("a reply");
        assert_eq!(refused.metadata.response_code, ResponseCode::NXDomain);
        assert_eq!(
            asked.load(Ordering::SeqCst),
            1,
            "the refused name never left"
        );
        let noted = egress::refused(&database, &project.id, 10)
            .await
            .expect("read");
        assert_eq!(noted[0].name, "leak.attacker.example");
    }

    #[tokio::test]
    async fn only_a_projects_container_is_answered() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "shop").await.expect("project");
        let index = projects::ensure_network_index(&database, &project.id)
            .await
            .expect("a network");
        let deployer = Deployer::new(
            Arc::new(database.clone()),
            &crate::infra::config::Config::default(),
        );
        let (upstream, asked) = upstream().await;
        let query = question("api.example.com.");

        for stranger in [
            IpAddr::from([203, 0, 113, 9]),
            IpAddr::from([10, 42, index, 1]),
            IpAddr::from([10, 42, index + 1, 5]),
            IpAddr::from([10, 42, 0, 2]),
        ] {
            assert_eq!(
                answer(&deployer, &[upstream], &query, stranger, Over::Udp).await,
                None,
                "{stranger}"
            );
        }
        assert_eq!(asked.load(Ordering::SeqCst), 0);
    }
}
