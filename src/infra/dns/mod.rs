//! The zones this node answers DNS for, built from the rows and held where
//! the listener reads them.
//!
//! Replaced whole on every change, like the route table: a query is answered
//! from what is in memory, and a lock on that would put every answer behind
//! the rare write.

pub mod server;

use std::collections::BTreeMap;
use std::sync::Arc;

use arc_swap::ArcSwap;
use hickory_server::net::runtime::TokioRuntimeProvider;
use hickory_server::proto::rr::rdata::{self, caa::KeyValue};
use hickory_server::proto::rr::{LowerName, Name, RData, Record};
use hickory_server::store::in_memory::InMemoryZoneHandler;
use hickory_server::zone_handler::{AxfrPolicy, Catalog, ZoneHandler, ZoneType};
use tokio::sync::Notify;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::dns_record::DnsRecord;
use crate::domain::entities::dns_zone::DnsZone;
use crate::domain::values::record_data::RecordData;
use crate::infra::repositories::zones;

const AUTHORITY_TTL: u32 = 3600;
const REFRESH: i32 = 3600;
const RETRY: i32 = 600;
const EXPIRE: i32 = 1_209_600;

/// One zone as it is served: what the rows say, with the names of the
/// nodes that answer for it already resolved to the names they publish.
#[derive(Debug, Clone)]
pub struct Served {
    pub zone: DnsZone,
    pub records: Vec<DnsRecord>,
    pub name_servers: Vec<String>,
}

pub struct Zones {
    catalog: ArcSwap<Catalog>,
    served: ArcSwap<Vec<Served>>,
    listening: ArcSwap<Vec<server::Listening>>,
    changed: Notify,
}

impl Default for Zones {
    fn default() -> Self {
        Self {
            catalog: ArcSwap::from_pointee(Catalog::new()),
            served: ArcSwap::from_pointee(Vec::new()),
            listening: ArcSwap::from_pointee(Vec::new()),
            changed: Notify::new(),
        }
    }
}

impl Zones {
    /// Where the listener is answering now, and where it could not.
    pub fn listening(&self) -> Arc<Vec<server::Listening>> {
        self.listening.load_full()
    }

    fn set_listening(&self, listening: Vec<server::Listening>) {
        self.listening.store(Arc::new(listening));
    }

    pub fn catalog(&self) -> Arc<Catalog> {
        self.catalog.load_full()
    }

    pub fn served(&self) -> Arc<Vec<Served>> {
        self.served.load_full()
    }

    pub async fn changed(&self) {
        self.changed.notified().await
    }

    /// Read the rows again and answer from them.
    pub async fn reload(&self, database: &SqliteDatabase) {
        let served = match what_this_node_serves(database).await {
            Ok(served) => served,
            Err(error) => {
                tracing::warn!(%error, "could not read the zones; still answering from the last ones");
                return;
            }
        };
        let mut catalog = Catalog::new();
        for zone in &served {
            match handler(zone) {
                Ok(handler) => {
                    let origin = LowerName::new(&handler.0);
                    catalog.upsert(origin, vec![Arc::new(handler.1) as Arc<dyn ZoneHandler>]);
                }
                Err(reason) => {
                    tracing::warn!(zone = %zone.zone.name, %reason, "a zone this node cannot serve");
                }
            }
        }
        self.catalog.store(Arc::new(catalog));
        self.served.store(Arc::new(served));
        self.changed.notify_one();
    }
}

/// Ask the running node's zone table to read the rows again, wherever the
/// caller is. A process with none registered — every test, and the CLI —
/// has no listener to tell.
pub async fn reload(container: &wabot::prelude::Container, database: &SqliteDatabase) {
    match container.try_resolve::<Zones>() {
        Ok(zones) => zones.reload(database).await,
        Err(_) => tracing::debug!("no zone table here; nothing is answering DNS"),
    }
}

/// The name each node publishes about itself, which is what a zone lists
/// as its name server: this node's own domain, and another's endpoint
/// without the port. A node with neither has no name to give.
pub async fn published_names(
    database: &SqliteDatabase,
) -> Result<BTreeMap<String, String>, String> {
    let nodes = crate::infra::network::all(database)
        .await
        .map_err(|error| error.to_string())?;
    let own_domain = crate::infra::node::settings::domain(database).await;
    Ok(nodes
        .iter()
        .filter_map(|node| {
            let name = match node.is_self {
                true => own_domain.clone(),
                false => node
                    .endpoint
                    .as_deref()
                    .map(|endpoint| endpoint.rsplit_once(':').map_or(endpoint, |(host, _)| host))
                    .map(str::to_string),
            }?;
            Some((node.id.clone(), name))
        })
        .collect())
}

/// The zones whose servers include this node, and the ones it was sent.
pub async fn what_this_node_serves(database: &SqliteDatabase) -> Result<Vec<Served>, String> {
    let me = crate::infra::network::all(database)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|node| node.is_self)
        .map(|node| node.id);
    let published = published_names(database).await?;

    let mut served = Vec::new();
    for zone in zones::all(database)
        .await
        .map_err(|error| error.to_string())?
    {
        let servers = zones::servers(database, &zone.id)
            .await
            .map_err(|error| error.to_string())?;
        let answers_here = !zone.is_ours() || me.as_ref().is_some_and(|me| servers.contains(me));
        if !answers_here {
            continue;
        }
        let records = zones::records(database, &zone)
            .await
            .map_err(|error| error.to_string())?;
        let name_servers = servers
            .iter()
            .filter_map(|node| published.get(node).cloned())
            .collect();
        served.push(Served {
            zone,
            records,
            name_servers,
        });
    }
    Ok(served)
}

fn handler(served: &Served) -> Result<(Name, InMemoryZoneHandler<TokioRuntimeProvider>), String> {
    let origin = absolute(served.zone.name.as_str())?;
    let serial = served.zone.serial;
    let mut handler = InMemoryZoneHandler::<TokioRuntimeProvider>::empty(
        origin.clone(),
        ZoneType::Primary,
        AxfrPolicy::Deny,
    );

    let primary = match served.name_servers.first() {
        Some(name) => absolute(name)?,
        None => origin.clone(),
    };
    let soa = rdata::SOA::new(
        primary,
        absolute(&served.zone.hostmaster())?,
        serial,
        REFRESH,
        RETRY,
        EXPIRE,
        served.zone.default_ttl,
    );
    handler.upsert_mut(
        Record::from_rdata(origin.clone(), AUTHORITY_TTL, RData::SOA(soa)),
        serial,
    );
    for name in &served.name_servers {
        let ns = RData::NS(rdata::NS(absolute(name)?));
        handler.upsert_mut(
            Record::from_rdata(origin.clone(), AUTHORITY_TTL, ns),
            serial,
        );
    }

    for record in &served.records {
        let owner = absolute(&record.name.fqdn(&served.zone.name))?;
        match rdata_of(&record.data) {
            Ok(data) => {
                handler.upsert_mut(Record::from_rdata(owner, record.ttl, data), serial);
            }
            Err(reason) => tracing::warn!(record = %record.id, %reason, "a record left out"),
        }
    }
    Ok((origin, handler))
}

fn rdata_of(data: &RecordData) -> Result<RData, String> {
    Ok(match data {
        RecordData::A(address) => RData::A(rdata::A(*address)),
        RecordData::Aaaa(address) => RData::AAAA(rdata::AAAA(*address)),
        RecordData::Cname(target) => RData::CNAME(rdata::CNAME(absolute(target)?)),
        RecordData::Mx { priority, exchange } => {
            RData::MX(rdata::MX::new(*priority, absolute(exchange)?))
        }
        RecordData::Txt(text) => RData::TXT(rdata::TXT::new(RecordData::text_strings(text))),
        RecordData::Srv {
            priority,
            weight,
            port,
            target,
        } => RData::SRV(rdata::SRV::new(
            *priority,
            *weight,
            *port,
            absolute(target)?,
        )),
        RecordData::Caa {
            critical,
            tag,
            value,
        } => {
            let mut parts = value.split(';').map(str::trim);
            let issuer = match parts.next().filter(|issuer| !issuer.is_empty()) {
                Some(issuer) => Some(absolute(issuer)?),
                None => None,
            };
            let options: Vec<KeyValue> = parts
                .filter_map(|option| option.split_once('='))
                .map(|(key, value)| KeyValue::new(key.trim(), value.trim()))
                .collect();
            RData::CAA(match tag.as_str() {
                "issuewild" => rdata::CAA::new_issuewild(*critical, issuer, options),
                _ => rdata::CAA::new_issue(*critical, issuer, options),
            })
        }
    })
}

/// Ask `at` for a zone's SOA the way a resolver would, and hand back the
/// serial if the answer is authoritative.
///
/// Asking the thing rather than the setting: a switch that is on and a
/// listener that took the port are two different facts, and a firewall in
/// front of either is a third.
pub async fn probe(at: std::net::SocketAddr, zone: &str) -> Result<u32, String> {
    use hickory_server::proto::op::{Message, Query, ResponseCode};
    use hickory_server::proto::rr::RecordType;

    let mut question = Message::query();
    question.add_query(Query::query(absolute(zone)?, RecordType::SOA));
    let asked = question.to_vec().map_err(|error| error.to_string())?;

    let local: std::net::SocketAddr = match at {
        std::net::SocketAddr::V4(_) => "0.0.0.0:0".parse().map_err(|_| "no local address")?,
        std::net::SocketAddr::V6(_) => "[::]:0".parse().map_err(|_| "no local address")?,
    };
    let socket = tokio::net::UdpSocket::bind(local)
        .await
        .map_err(|error| error.to_string())?;
    socket
        .send_to(&asked, at)
        .await
        .map_err(|error| error.to_string())?;
    let mut buffer = vec![0u8; 4096];
    let read = tokio::time::timeout(std::time::Duration::from_secs(2), socket.recv(&mut buffer))
        .await
        .map_err(|_| "no answer within two seconds".to_string())?
        .map_err(|error| error.to_string())?;
    let answer = Message::from_vec(&buffer[..read]).map_err(|error| error.to_string())?;

    if answer.metadata.response_code != ResponseCode::NoError {
        return Err(format!("answered {}", answer.metadata.response_code));
    }
    if !answer.metadata.authoritative {
        return Err("answered, but not as the authority for it".into());
    }
    answer
        .answers
        .iter()
        .find_map(|record| match &record.data {
            RData::SOA(soa) => Some(soa.serial),
            _ => None,
        })
        .ok_or_else(|| "answered without the zone's SOA".to_string())
}

fn absolute(name: &str) -> Result<Name, String> {
    let name = name.trim_end_matches('.');
    Name::from_ascii(format!("{name}.")).map_err(|error| format!("{name}: {error}"))
}
