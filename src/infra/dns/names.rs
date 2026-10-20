//! The names a node answers DNS as.
//!
//! A node's own are set by its operator and default to its domain; another
//! node's are what it said on its report or the roster relayed. A zone lists
//! each of its servers under these, so they have to agree everywhere the
//! zone is served — which is why they travel rather than being typed on
//! the node that administers the zone.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;

use wabot::prelude::Container;
use wabot::sqlite::{SqliteDatabase, SqliteResult};

use crate::domain::values::name_server::NameServerName;
use crate::infra::node::settings;

const DECLARED: &str = "dns.name_servers";

pub async fn declared(database: &SqliteDatabase) -> Vec<String> {
    match settings::read(database, DECLARED).await {
        Ok(Some(said)) => split(&said),
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(%error, "could not read the names this node answers DNS as");
            Vec::new()
        }
    }
}

pub async fn declare(database: &SqliteDatabase, names: &[NameServerName]) -> SqliteResult<()> {
    let said: Vec<&str> = names.iter().map(NameServerName::as_str).collect();
    settings::write(database, DECLARED, &said.join(",")).await
}

pub async fn of_this_node(database: &SqliteDatabase) -> Vec<String> {
    let declared = declared(database).await;
    if !declared.is_empty() {
        return declared;
    }
    settings::domain(database).await.into_iter().collect()
}

/// Keep what another node said it answers as, and say whether that moved
/// anything — a report arrives every fifteen seconds, and re-sending every
/// zone on each one would be a storm of errands saying nothing new.
pub async fn remember(
    database: &SqliteDatabase,
    node_id: &str,
    said: &[String],
) -> SqliteResult<bool> {
    let (node_id, said) = (node_id.to_string(), NameServerName::kept(said).join(","));
    database
        .write(move |connection| {
            let changed = connection.execute(
                "UPDATE node SET \"name_servers\" = ?2 \
                 WHERE \"id\" = ?1 AND \"name_servers\" IS NOT ?2",
                (node_id, said),
            )?;
            Ok(changed > 0)
        })
        .await
}

pub async fn said_by_others(
    database: &SqliteDatabase,
) -> SqliteResult<BTreeMap<String, Vec<String>>> {
    database
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT \"id\", \"name_servers\" FROM node \
                 WHERE \"name_servers\" IS NOT NULL AND \"is_self\" = 0",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut said = BTreeMap::new();
            for row in rows {
                let (id, names) = row?;
                said.insert(id, split(&names));
            }
            Ok(said)
        })
        .await
}

#[derive(Debug, Clone)]
pub struct Pointing {
    pub name: String,
    pub addresses: Vec<IpAddr>,
    pub here: bool,
}

/// Each name this node answers as, looked up the way a resolver would.
///
/// "Here" is an address this machine holds or one its own domain resolves
/// to: behind a provider's NAT the public address is on no interface, and
/// the domain is what already says where the world reaches this node.
pub async fn where_each_points(database: &SqliteDatabase) -> Vec<Pointing> {
    let mut here: BTreeSet<IpAddr> = crate::infra::network::lan::held()
        .into_iter()
        .map(|held| IpAddr::V4(held.address))
        .collect();
    if let Some(domain) = settings::domain(database).await {
        here.extend(crate::infra::deploy::dns::lookup(&domain).await);
    }
    let mut pointing = Vec::new();
    for name in of_this_node(database).await {
        let addresses = crate::infra::deploy::dns::lookup(&name).await;
        pointing.push(Pointing {
            here: reaches(&addresses, &here),
            addresses: addresses.into_iter().collect(),
            name,
        });
    }
    pointing
}

/// What a change of names moves: the NS records this node serves, and every
/// zone administered here, which is sent to its other servers by name.
pub async fn changed(container: &Container, database: &SqliteDatabase) {
    super::reload(container, database).await;
    super::dispatch::tell_all(database).await;
}

fn reaches(addresses: &BTreeSet<IpAddr>, here: &BTreeSet<IpAddr>) -> bool {
    !addresses.is_empty() && addresses.is_subset(here)
}

fn split(said: &str) -> Vec<String> {
    said.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::Config;
    use crate::infra::network::{Kind, Node};

    fn addresses(list: &[&str]) -> BTreeSet<IpAddr> {
        list.iter()
            .map(|address| address.parse().expect("an address"))
            .collect()
    }

    /// A name that also leads somewhere else sends some resolvers there.
    #[test]
    fn a_name_points_here_only_when_everything_it_leads_to_is_here() {
        let here = addresses(&["209.50.48.176", "10.0.0.4"]);
        assert!(reaches(&addresses(&["209.50.48.176"]), &here));
        assert!(!reaches(
            &addresses(&["209.50.48.176", "198.51.100.9"]),
            &here
        ));
        assert!(!reaches(&addresses(&[]), &here));
    }

    #[tokio::test]
    async fn this_node_answers_as_its_domain_until_its_operator_names_it() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::node::settings::set_domain(&database, Some("node-0.network.nubebasura.lat"))
            .await
            .expect("domain");
        assert_eq!(
            of_this_node(&database).await,
            ["node-0.network.nubebasura.lat"]
        );

        let names =
            NameServerName::all(&["ns0.nubebasura.lat".into(), "ns1.nubebasura.lat".into()])
                .expect("names");
        declare(&database, &names).await.expect("declared");
        assert_eq!(
            of_this_node(&database).await,
            ["ns0.nubebasura.lat", "ns1.nubebasura.lat"]
        );

        declare(&database, &[]).await.expect("cleared");
        assert_eq!(
            of_this_node(&database).await,
            ["node-0.network.nubebasura.lat"]
        );
    }

    #[tokio::test]
    async fn what_another_node_says_is_news_once() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::network::ensure_self(&database, &Config::default())
            .await
            .expect("self");
        crate::infra::network::save(
            &database,
            &Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-peer".into(),
                name: "node-1.example.net".into(),
                kind: Kind::Public,
                endpoint: Some("node-1.example.net:443".into()),
                public_key: None,
                overlay_ip: Some("10.42.0.8".into()),
                is_self: false,
                last_seen_at: None,
                allows: Vec::new(),
                ca_pem: None,
            },
        )
        .await
        .expect("peer");
        assert!(said_by_others(&database).await.expect("read").is_empty());

        let said = vec!["ns2.example.net".to_string(), "ns3.example.net".to_string()];
        assert!(remember(&database, "nd-peer", &said)
            .await
            .expect("written"));
        assert!(!remember(&database, "nd-peer", &said)
            .await
            .expect("written"));
        assert_eq!(
            said_by_others(&database)
                .await
                .expect("read")
                .get("nd-peer"),
            Some(&said)
        );
    }
}
