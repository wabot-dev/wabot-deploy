//! Port 53: whether to listen, where, and how much to answer one asker.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use hickory_server::net::runtime::Time;
use hickory_server::net::xfer::Protocol;
use hickory_server::proto::op::{Header, HeaderCounts, MessageType, Metadata, OpCode};
use hickory_server::server::{Request, RequestHandler, ResponseHandler, ResponseInfo};
use hickory_server::Server;
use serde::Serialize;
use wabot::lifecycle::Cancel;
use wabot::sqlite::SqliteDatabase;

use super::Zones;
use crate::domain::values::capability::Capability;
use crate::infra::config::Config;

const PER_SECOND: f64 = 20.0;
const BURST: f64 = 40.0;
const TCP_IDLE: Duration = Duration::from_secs(10);
const TCP_RESPONSES: usize = 32;
const RECHECK: Duration = Duration::from_secs(60);

/// One address this node tried to answer DNS on, and what happened.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Listening {
    pub address: SocketAddr,
    pub refused: Option<String>,
}

/// Answers from the zone table, after asking whether this source has had
/// its share.
///
/// **The share is per source, over UDP only.** A spoofed source is the
/// whole of an amplification attack, and UDP is the only way to spoof one;
/// a resolver that is cut off retries over TCP, which a spoofed source
/// cannot complete.
pub struct Answering {
    zones: Arc<Zones>,
    budget: Mutex<HashMap<IpAddr, Bucket>>,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

impl Answering {
    pub fn new(zones: Arc<Zones>) -> Self {
        Self {
            zones,
            budget: Mutex::new(HashMap::new()),
        }
    }

    fn allowed(&self, source: IpAddr) -> bool {
        let mut budget = self
            .budget
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if budget.len() > 10_000 {
            budget.retain(|_, bucket| bucket.at.elapsed() < Duration::from_secs(10));
        }
        let now = Instant::now();
        let bucket = budget.entry(source).or_insert(Bucket {
            tokens: BURST,
            at: now,
        });
        bucket.tokens =
            (bucket.tokens + now.duration_since(bucket.at).as_secs_f64() * PER_SECOND).min(BURST);
        bucket.at = now;
        match bucket.tokens >= 1.0 {
            true => {
                bucket.tokens -= 1.0;
                true
            }
            false => false,
        }
    }
}

#[async_trait::async_trait]
impl RequestHandler for Answering {
    async fn handle_request<R: ResponseHandler, T: Time>(
        &self,
        request: &Request,
        response_handle: R,
    ) -> ResponseInfo {
        if request.protocol() == Protocol::Udp && !self.allowed(request.src().ip()) {
            return unanswered(request);
        }
        self.zones
            .catalog()
            .handle_request::<R, T>(request, response_handle)
            .await
    }
}

fn unanswered(request: &Request) -> ResponseInfo {
    let (id, op_code) = match request.request_info() {
        Ok(info) => (info.metadata.id, info.metadata.op_code),
        Err(_) => (0, OpCode::Query),
    };
    ResponseInfo::from(Header {
        metadata: Metadata::new(id, MessageType::Response, op_code),
        counts: HeaderCounts::default(),
    })
}

/// Listen while this node provides `dns` and serves a zone, and not
/// otherwise; follow what changes without a restart.
pub async fn serve(
    zones: Arc<Zones>,
    database: Arc<SqliteDatabase>,
    config: Config,
    cancel: Cancel,
) -> std::io::Result<()> {
    let forgotten = super::forget_ungranted(&database).await;
    if !forgotten.is_empty() {
        tracing::info!(
            zones = forgotten.join(", "),
            "let go of zones held for nodes no longer granted dns"
        );
    }
    zones.refresh(&database).await;
    super::dispatch::tell_all(&database).await;
    let mut running: Option<(Vec<SocketAddr>, Server<Answering>)> = None;
    loop {
        let wanted = wanted(&zones, &database, &config).await;
        let current = running.as_ref().map(|(addresses, _)| addresses.clone());
        if current.as_ref() != Some(&wanted) && !(current.is_none() && wanted.is_empty()) {
            if let Some((_, mut server)) = running.take() {
                if let Err(error) = server.shutdown_gracefully().await {
                    tracing::warn!(%error, "the DNS listener did not stop cleanly");
                }
            }
            zones.set_listening(Vec::new());
            if !wanted.is_empty() {
                let (server, listening) = start(zones.clone(), &wanted).await;
                zones.set_listening(listening);
                running = Some((wanted, server));
            }
        }
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = zones.changed() => {}
            _ = tokio::time::sleep(RECHECK) => {
                zones.refresh(&database).await;
            }
        }
    }
    if let Some((_, mut server)) = running.take() {
        let _ = server.shutdown_gracefully().await;
    }
    Ok(())
}

async fn wanted(zones: &Zones, database: &SqliteDatabase, config: &Config) -> Vec<SocketAddr> {
    if zones.served().is_empty()
        || !crate::infra::network::capability::provides(database, Capability::Dns).await
    {
        return Vec::new();
    }
    addresses(config)
}

/// Where the listener binds when it listens: the configured addresses, or
/// every one this machine holds that is worth telling anybody about.
pub fn addresses(config: &Config) -> Vec<SocketAddr> {
    let mut addresses: Vec<IpAddr> = match config.dns.listen.is_empty() {
        true => crate::infra::network::lan::held()
            .into_iter()
            .map(|held| IpAddr::V4(held.address))
            .collect(),
        false => config.dns.listen.clone(),
    };
    addresses.sort();
    addresses.dedup();
    addresses
        .into_iter()
        .map(|address| SocketAddr::new(address, config.dns.port))
        .collect()
}

async fn start(zones: Arc<Zones>, addresses: &[SocketAddr]) -> (Server<Answering>, Vec<Listening>) {
    let mut server = Server::new(Answering::new(zones));
    let mut listening = Vec::new();
    for address in addresses {
        let bound = async {
            let udp = tokio::net::UdpSocket::bind(address).await?;
            let tcp = tokio::net::TcpListener::bind(address).await?;
            Ok::<_, std::io::Error>((udp, tcp))
        }
        .await;
        match bound {
            Ok((udp, tcp)) => {
                server.register_socket(udp);
                server.register_listener(tcp, TCP_IDLE, TCP_RESPONSES);
                tracing::info!(%address, "answering DNS");
                listening.push(Listening {
                    address: *address,
                    refused: None,
                });
            }
            Err(error) => {
                tracing::warn!(%address, %error, "could not answer DNS there");
                listening.push(Listening {
                    address: *address,
                    refused: Some(error.to_string()),
                });
            }
        }
    }
    (server, listening)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::record_data::{RecordData, RecordKind};
    use crate::domain::values::record_name::RecordName;
    use crate::domain::values::zone_name::ZoneName;
    use crate::infra::repositories::zones;
    use hickory_server::proto::op::{Message, Query, ResponseCode};
    use hickory_server::proto::rr::{Name, RData, RecordType};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const ADDRESS: &str = "209.50.48.176";

    struct Listener {
        udp: SocketAddr,
        tcp: SocketAddr,
        _server: Server<Answering>,
    }

    async fn node(
        records: &[(&str, RecordKind, &str, Option<&str>)],
    ) -> (Arc<Zones>, SqliteDatabase) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::node::settings::set_domain(&database, Some("node-0.example.net"))
            .await
            .expect("domain");
        let me = crate::infra::network::ensure_self(&database, &Config::default())
            .await
            .expect("self");
        let name = ZoneName::parse("teresaasprilla.com").expect("zone");
        let zone = zones::create(
            &database,
            zones::NewZone {
                name: &name,
                tenant_id: None,
                servers: &[me.id],
            },
        )
        .await
        .expect("zone");
        for (at, kind, value, priority) in records {
            zones::add_record(
                &database,
                &zone,
                RecordName::parse(at, &name).expect("name"),
                RecordData::parse(*kind, value, *priority).expect("data"),
                300,
            )
            .await
            .expect("record");
        }
        let table = Arc::new(Zones::default());
        table.reload(&database).await;
        (table, database)
    }

    async fn listen(zones: Arc<Zones>) -> Listener {
        let udp = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("udp");
        let udp_at = udp.local_addr().expect("udp address");
        let tcp = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("tcp");
        let tcp_at = tcp.local_addr().expect("tcp address");
        let mut server = Server::new(Answering::new(zones));
        server.register_socket(udp);
        server.register_listener(tcp, TCP_IDLE, TCP_RESPONSES);
        Listener {
            udp: udp_at,
            tcp: tcp_at,
            _server: server,
        }
    }

    fn question(name: &str, kind: RecordType) -> Vec<u8> {
        let mut message = Message::query();
        message.add_query(Query::query(Name::from_ascii(name).expect("name"), kind));
        message.to_vec().expect("encode")
    }

    async fn over_udp(at: SocketAddr, name: &str, kind: RecordType) -> Message {
        let socket = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind");
        socket
            .send_to(&question(name, kind), at)
            .await
            .expect("send");
        let mut buffer = vec![0u8; 4096];
        let read = tokio::time::timeout(Duration::from_secs(2), socket.recv(&mut buffer))
            .await
            .expect("an answer in time")
            .expect("recv");
        Message::from_vec(&buffer[..read]).expect("decode")
    }

    async fn over_tcp(at: SocketAddr, name: &str, kind: RecordType) -> Message {
        let mut stream = tokio::net::TcpStream::connect(at).await.expect("connect");
        let asked = question(name, kind);
        stream
            .write_all(&(asked.len() as u16).to_be_bytes())
            .await
            .expect("length");
        stream.write_all(&asked).await.expect("question");
        let mut length = [0u8; 2];
        stream.read_exact(&mut length).await.expect("length back");
        let mut answer = vec![0u8; u16::from_be_bytes(length) as usize];
        stream.read_exact(&mut answer).await.expect("answer");
        Message::from_vec(&answer).expect("decode")
    }

    #[tokio::test]
    async fn an_answer_for_its_own_zone_is_authoritative() {
        let (zones, _database) = node(&[("@", RecordKind::A, ADDRESS, None)]).await;
        let listener = listen(zones).await;

        let answer = over_udp(listener.udp, "teresaasprilla.com.", RecordType::A).await;
        assert_eq!(answer.metadata.response_code, ResponseCode::NoError);
        assert!(
            answer.metadata.authoritative,
            "an authoritative server says so"
        );
        assert!(
            !answer.metadata.recursion_available,
            "and it recurses for nobody"
        );
        assert!(
            matches!(&answer.answers[0].data, RData::A(address) if address.0.to_string() == ADDRESS)
        );
    }

    /// An open resolver is an amplifier anybody can point at a victim.
    #[tokio::test]
    async fn a_name_outside_its_zones_is_refused() {
        let (zones, _database) = node(&[("@", RecordKind::A, ADDRESS, None)]).await;
        let listener = listen(zones).await;

        let answer = over_udp(listener.udp, "google.com.", RecordType::A).await;
        assert_eq!(answer.metadata.response_code, ResponseCode::Refused);
        assert!(answer.answers.is_empty());
    }

    /// Without the SOA a resolver cannot tell how long to remember that a
    /// name does not exist, and asks again on every lookup.
    #[tokio::test]
    async fn a_missing_name_or_type_carries_the_soa() {
        let (zones, _database) = node(&[("@", RecordKind::A, ADDRESS, None)]).await;
        let listener = listen(zones).await;

        let missing = over_udp(listener.udp, "nothere.teresaasprilla.com.", RecordType::A).await;
        assert_eq!(missing.metadata.response_code, ResponseCode::NXDomain);
        assert!(missing
            .authorities
            .iter()
            .any(|record| matches!(record.data, RData::SOA(_))));

        let no_data = over_udp(listener.udp, "teresaasprilla.com.", RecordType::AAAA).await;
        assert_eq!(no_data.metadata.response_code, ResponseCode::NoError);
        assert!(no_data.answers.is_empty());
        assert!(no_data
            .authorities
            .iter()
            .any(|record| matches!(record.data, RData::SOA(_))));
    }

    #[tokio::test]
    async fn the_apex_names_the_nodes_that_answer_for_it() {
        let (zones, _database) = node(&[]).await;
        let listener = listen(zones).await;

        let answer = over_udp(listener.udp, "teresaasprilla.com.", RecordType::NS).await;
        let names: Vec<String> = answer
            .answers
            .iter()
            .filter_map(|record| match &record.data {
                RData::NS(ns) => Some(ns.0.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["node-0.example.net.".to_string()]);
    }

    #[tokio::test]
    async fn a_cname_is_followed_and_a_wildcard_answers() {
        let (zones, _database) = node(&[
            ("@", RecordKind::A, ADDRESS, None),
            ("www", RecordKind::Cname, "teresaasprilla.com", None),
            ("*.blog", RecordKind::A, "203.0.113.9", None),
        ])
        .await;
        let listener = listen(zones).await;

        let www = over_udp(listener.udp, "www.teresaasprilla.com.", RecordType::A).await;
        assert!(www
            .answers
            .iter()
            .any(|record| matches!(record.data, RData::CNAME(_))));
        assert!(www
            .answers
            .iter()
            .any(|record| matches!(record.data, RData::A(_))));

        let any = over_udp(
            listener.udp,
            "anything.blog.teresaasprilla.com.",
            RecordType::A,
        )
        .await;
        assert!(
            matches!(&any.answers[0].data, RData::A(address) if address.0.to_string() == "203.0.113.9")
        );
    }

    /// A DKIM key is longer than a plain UDP answer may be. The answer says
    /// it was cut, and the same question over TCP gets the whole of it.
    #[tokio::test]
    async fn a_long_txt_is_cut_over_udp_and_whole_over_tcp() {
        let key = format!("k=rsa; p={}", "A".repeat(1500));
        let (zones, _database) = node(&[("k1._domainkey.mg", RecordKind::Txt, &key, None)]).await;
        let listener = listen(zones).await;
        let name = "k1._domainkey.mg.teresaasprilla.com.";

        let cut = over_udp(listener.udp, name, RecordType::TXT).await;
        assert!(
            cut.metadata.truncation,
            "a plain UDP answer stops at 512 bytes"
        );

        let whole = over_tcp(listener.tcp, name, RecordType::TXT).await;
        let text: String = whole
            .answers
            .iter()
            .filter_map(|record| match &record.data {
                RData::TXT(txt) => Some(
                    txt.txt_data
                        .iter()
                        .map(|part| String::from_utf8_lossy(part).to_string())
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect();
        assert_eq!(text, key);
    }

    /// What `doctor` asks, and what it is told: the serial on a zone this
    /// node answers for, and a reason on one it does not.
    #[tokio::test]
    async fn the_probe_hears_what_a_resolver_hears() {
        let (zones, database) = node(&[("@", RecordKind::A, ADDRESS, None)]).await;
        let listener = listen(zones).await;

        let zone = zones::find_by_name(&database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("zone");
        assert_eq!(
            crate::infra::dns::probe(listener.udp, "teresaasprilla.com").await,
            Ok(zone.serial)
        );
        assert!(crate::infra::dns::probe(listener.udp, "google.com")
            .await
            .is_err());
    }

    #[test]
    fn one_source_gets_its_share_and_no_more() {
        let answering = Answering::new(Arc::new(Zones::default()));
        let source: IpAddr = "198.51.100.7".parse().expect("ip");
        let answered = (0..100).filter(|_| answering.allowed(source)).count();
        assert!(
            (BURST as usize..BURST as usize + 3).contains(&answered),
            "{answered} of 100 answered at once"
        );
        assert!(
            answering.allowed("198.51.100.8".parse().expect("ip")),
            "another source has its own"
        );
    }

    #[tokio::test]
    async fn the_port_opens_only_for_a_zone_on_a_node_that_said_yes() {
        let (zones, database) = node(&[("@", RecordKind::A, ADDRESS, None)]).await;
        let mut config = Config::default();
        config.dns.port = 5300;
        config.dns.listen = vec!["127.0.0.1".parse().expect("ip")];

        assert!(
            wanted(&zones, &database, &config).await.is_empty(),
            "off until turned on"
        );

        crate::infra::network::capability::set_provides(&database, Capability::Dns, true)
            .await
            .expect("on");
        assert_eq!(
            wanted(&zones, &database, &config).await,
            vec!["127.0.0.1:5300".parse::<SocketAddr>().expect("address")]
        );

        let empty = Arc::new(Zones::default());
        assert!(
            wanted(&empty, &database, &config).await.is_empty(),
            "no zone, no port"
        );
    }
}
