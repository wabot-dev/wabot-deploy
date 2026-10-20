//! Where the parent zone sends resolvers for a zone.
//!
//! **Asked of the parent, not of a resolver.** A resolver remembers, and
//! one whose delegation is broken answers SERVFAIL and says nothing about
//! why. The parent's own servers, asked without recursion, answer with
//! exactly what the registrar wrote — `ns1.linode.com` for a zone whose
//! Linode account had gone, which is the failure this exists to name.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use hickory_server::proto::op::{Message, Query, ResponseCode};
use hickory_server::proto::rr::{Name, RData, RecordType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const WAIT: Duration = Duration::from_secs(3);
const FALLBACK_RESOLVERS: [&str; 2] = ["1.1.1.1", "8.8.8.8"];

/// One DNS question to one server. A trait so the walk below can be tested
/// with answers written down rather than with the internet.
#[async_trait::async_trait]
pub trait Asker: Send + Sync {
    async fn ask(
        &self,
        server: IpAddr,
        name: &str,
        kind: RecordType,
        recurse: bool,
    ) -> Result<Message, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Delegated(Vec<String>),
    Unregistered,
    Unanswered(String),
}

/// The name servers the parent zone lists for `zone`.
pub async fn delegated(asker: &dyn Asker, resolvers: &[IpAddr], zone: &str) -> Outcome {
    let (parent, names) = match parent_of(asker, resolvers, zone).await {
        Ok(found) => found,
        Err(reason) => return Outcome::Unanswered(reason),
    };
    let servers = match addresses_of(asker, resolvers, &parent, &names).await {
        Ok(servers) => servers,
        Err(reason) => return Outcome::Unanswered(reason),
    };

    let mut last = format!("no server of {parent} answered");
    for server in servers.iter().take(3) {
        match asker.ask(*server, zone, RecordType::NS, false).await {
            Ok(answer) if answer.metadata.response_code == ResponseCode::NXDomain => {
                return Outcome::Unregistered;
            }
            Ok(answer) if answer.metadata.response_code == ResponseCode::NoError => {
                let names = ns_names(&answer, zone);
                return match names.is_empty() {
                    true => Outcome::Unregistered,
                    false => Outcome::Delegated(names),
                };
            }
            Ok(answer) => last = format!("{server} answered {}", answer.metadata.response_code),
            Err(reason) => last = reason,
        }
    }
    Outcome::Unanswered(last)
}

/// The resolvers this machine already uses, read from where every program
/// reads them. Two public ones only when the file names none.
pub fn system_resolvers() -> Vec<IpAddr> {
    let listed: Vec<IpAddr> = std::fs::read_to_string("/etc/resolv.conf")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().strip_prefix("nameserver"))
        .filter_map(|address| address.trim().split('%').next()?.parse().ok())
        .collect();
    match listed.is_empty() {
        false => listed,
        true => FALLBACK_RESOLVERS
            .iter()
            .filter_map(|address| address.parse().ok())
            .collect(),
    }
}

/// The closest enclosing zone, and its name servers: the first ancestor
/// with NS records of its own. Usually the TLD; `co.uk`, or a zone
/// delegated from another, where it is not.
///
/// **NS and not SOA**, because every zone cut has NS records and not every
/// resolver answers a SOA question: the one Lima gives a VM answers `com
/// SOA` with nothing at all, and `com NS` correctly. Measured on the VM node.
async fn parent_of(
    asker: &dyn Asker,
    resolvers: &[IpAddr],
    zone: &str,
) -> Result<(String, Vec<String>), String> {
    let labels: Vec<&str> = zone.split('.').collect();
    let mut last = format!("no zone above {zone} answered");
    for start in 1..labels.len() {
        let candidate = labels[start..].join(".");
        match recursively(asker, resolvers, &candidate, RecordType::NS).await {
            Ok(answer) => {
                let names: Vec<String> = answer
                    .answers
                    .iter()
                    .filter(|record| same(&record.name, &candidate))
                    .filter_map(|record| match &record.data {
                        RData::NS(ns) => {
                            Some(ns.0.to_string().trim_end_matches('.').to_ascii_lowercase())
                        }
                        _ => None,
                    })
                    .collect();
                if !names.is_empty() {
                    return Ok((candidate, names));
                }
            }
            Err(reason) => last = reason,
        }
    }
    Err(last)
}

async fn addresses_of(
    asker: &dyn Asker,
    resolvers: &[IpAddr],
    parent: &str,
    names: &[String],
) -> Result<Vec<IpAddr>, String> {
    let mut addresses = Vec::new();
    for name in names.iter().take(4) {
        if let Ok(found) = recursively(asker, resolvers, name, RecordType::A).await {
            addresses.extend(
                found
                    .answers
                    .iter()
                    .filter_map(|record| match &record.data {
                        RData::A(address) => Some(IpAddr::V4(address.0)),
                        _ => None,
                    }),
            );
        }
        if addresses.len() >= 3 {
            break;
        }
    }
    match addresses.is_empty() {
        true => Err(format!("the name servers of {parent} have no address")),
        false => Ok(addresses),
    }
}

async fn recursively(
    asker: &dyn Asker,
    resolvers: &[IpAddr],
    name: &str,
    kind: RecordType,
) -> Result<Message, String> {
    let mut last = "no resolver to ask".to_string();
    for resolver in resolvers {
        match asker.ask(*resolver, name, kind, true).await {
            Ok(answer) if answer.metadata.response_code == ResponseCode::NoError => {
                return Ok(answer)
            }
            Ok(answer) => {
                last = format!(
                    "{resolver} answered {} for {name}",
                    answer.metadata.response_code
                )
            }
            Err(reason) => last = reason,
        }
    }
    Err(last)
}

/// NS records for `owner`, wherever the answer put them: a referral carries
/// them in the authority section, an authoritative answer in the answers.
fn ns_names(answer: &Message, owner: &str) -> Vec<String> {
    answer
        .answers
        .iter()
        .chain(answer.authorities.iter())
        .filter(|record| same(&record.name, owner))
        .filter_map(|record| match &record.data {
            RData::NS(ns) => Some(ns.0.to_string().trim_end_matches('.').to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

fn same(name: &Name, other: &str) -> bool {
    name.to_string()
        .trim_end_matches('.')
        .eq_ignore_ascii_case(other.trim_end_matches('.'))
}

/// Over UDP, and over TCP when the answer says it was cut.
pub struct OverTheNetwork;

#[async_trait::async_trait]
impl Asker for OverTheNetwork {
    async fn ask(
        &self,
        server: IpAddr,
        name: &str,
        kind: RecordType,
        recurse: bool,
    ) -> Result<Message, String> {
        let mut question = Message::query();
        question.metadata.recursion_desired = recurse;
        let owner = Name::from_ascii(format!("{}.", name.trim_end_matches('.')))
            .map_err(|error| format!("{name}: {error}"))?;
        question.add_query(Query::query(owner, kind));
        let asked = question.to_vec().map_err(|error| error.to_string())?;
        let at = SocketAddr::new(server, 53);

        let answer = over_udp(at, &asked).await?;
        match answer.metadata.truncation {
            false => Ok(answer),
            true => over_tcp(at, &asked).await,
        }
    }
}

async fn over_udp(at: SocketAddr, asked: &[u8]) -> Result<Message, String> {
    let local = match at {
        SocketAddr::V4(_) => "0.0.0.0:0",
        SocketAddr::V6(_) => "[::]:0",
    };
    let socket = tokio::net::UdpSocket::bind(local)
        .await
        .map_err(|error| error.to_string())?;
    socket
        .send_to(asked, at)
        .await
        .map_err(|error| format!("{at}: {error}"))?;
    let mut buffer = vec![0u8; 4096];
    let read = tokio::time::timeout(WAIT, socket.recv(&mut buffer))
        .await
        .map_err(|_| format!("{} did not answer", at.ip()))?
        .map_err(|error| format!("{at}: {error}"))?;
    Message::from_vec(&buffer[..read]).map_err(|error| error.to_string())
}

async fn over_tcp(at: SocketAddr, asked: &[u8]) -> Result<Message, String> {
    let exchange = async {
        let mut stream = tokio::net::TcpStream::connect(at).await?;
        stream
            .write_all(&(asked.len() as u16).to_be_bytes())
            .await?;
        stream.write_all(asked).await?;
        let mut length = [0u8; 2];
        stream.read_exact(&mut length).await?;
        let mut answer = vec![0u8; u16::from_be_bytes(length) as usize];
        stream.read_exact(&mut answer).await?;
        Ok::<_, std::io::Error>(answer)
    };
    let answer = tokio::time::timeout(WAIT, exchange)
        .await
        .map_err(|_| format!("{} did not answer over TCP", at.ip()))?
        .map_err(|error| format!("{at}: {error}"))?;
    Message::from_vec(&answer).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_server::proto::op::OpCode;
    use hickory_server::proto::rr::rdata::{self, SOA};
    use hickory_server::proto::rr::Record;
    use std::collections::HashMap;

    const RESOLVER: &str = "192.0.2.53";
    const TLD_SERVER: &str = "192.0.2.1";

    /// Answers written down: (server, name, type, recursion) to a message.
    struct Written(HashMap<(String, String, RecordType, bool), Message>);

    #[async_trait::async_trait]
    impl Asker for Written {
        async fn ask(
            &self,
            server: IpAddr,
            name: &str,
            kind: RecordType,
            recurse: bool,
        ) -> Result<Message, String> {
            self.0
                .get(&(server.to_string(), name.to_string(), kind, recurse))
                .cloned()
                .ok_or_else(|| format!("{server} did not answer"))
        }
    }

    fn name(text: &str) -> Name {
        Name::from_ascii(format!("{text}.")).expect("name")
    }

    fn reply(code: ResponseCode, answers: Vec<Record>, authorities: Vec<Record>) -> Message {
        let mut message = Message::response(1, OpCode::Query);
        message.metadata.response_code = code;
        message.answers = answers;
        message.authorities = authorities;
        message
    }

    fn soa(owner: &str) -> Record {
        let data = SOA::new(
            name("a.gtld-servers.net"),
            name("nstld.verisign-grs.com"),
            1,
            1,
            1,
            1,
            1,
        );
        Record::from_rdata(name(owner), 900, RData::SOA(data))
    }

    fn ns(owner: &str, target: &str) -> Record {
        Record::from_rdata(name(owner), 172_800, RData::NS(rdata::NS(name(target))))
    }

    fn a(owner: &str, address: &str) -> Record {
        Record::from_rdata(
            name(owner),
            300,
            RData::A(rdata::A(address.parse().expect("ip"))),
        )
    }

    /// The world as it was on 2026-10-19: `.com` delegating
    /// teresaasprilla.com to Linode, whose servers no longer held it.
    fn the_internet(referral: Message) -> Written {
        Written(HashMap::from([
            (
                (RESOLVER.into(), "com".into(), RecordType::NS, true),
                reply(
                    ResponseCode::NoError,
                    vec![ns("com", "a.gtld-servers.net")],
                    vec![],
                ),
            ),
            (
                (
                    RESOLVER.into(),
                    "a.gtld-servers.net".into(),
                    RecordType::A,
                    true,
                ),
                reply(
                    ResponseCode::NoError,
                    vec![a("a.gtld-servers.net", TLD_SERVER)],
                    vec![],
                ),
            ),
            (
                (
                    TLD_SERVER.into(),
                    "teresaasprilla.com".into(),
                    RecordType::NS,
                    false,
                ),
                referral,
            ),
        ]))
    }

    fn resolvers() -> Vec<IpAddr> {
        vec![RESOLVER.parse().expect("ip")]
    }

    #[tokio::test]
    async fn the_parent_says_what_the_registrar_wrote() {
        let internet = the_internet(reply(
            ResponseCode::NoError,
            vec![],
            vec![
                ns("teresaasprilla.com", "ns1.linode.com"),
                ns("teresaasprilla.com", "ns2.linode.com"),
            ],
        ));
        assert_eq!(
            delegated(&internet, &resolvers(), "teresaasprilla.com").await,
            Outcome::Delegated(vec!["ns1.linode.com".into(), "ns2.linode.com".into()])
        );
    }

    #[tokio::test]
    async fn a_domain_the_parent_never_heard_of_is_unregistered() {
        let internet = the_internet(reply(ResponseCode::NXDomain, vec![], vec![soa("com")]));
        assert_eq!(
            delegated(&internet, &resolvers(), "teresaasprilla.com").await,
            Outcome::Unregistered
        );
    }

    #[tokio::test]
    async fn a_question_nobody_answers_says_who_did_not() {
        let internet = Written(HashMap::new());
        match delegated(&internet, &resolvers(), "teresaasprilla.com").await {
            Outcome::Unanswered(reason) => assert!(reason.contains(RESOLVER), "{reason}"),
            other => panic!("expected no answer, got {other:?}"),
        }
    }
}
