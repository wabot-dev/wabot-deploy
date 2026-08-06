//! What a service exposes.
//!
//! Three questions that used to be one column: what the process
//! listens on, whether that is reachable from outside the node, and
//! whether it answers HTTPS at a hostname. A port row answers all
//! three, and the common answer to the last two is "no".

use serde::Serialize;
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, PlatformError, PlatformResult};

/// Where published ports are allocated from.
///
/// Above the registered range and below the ephemeral one Linux hands
/// out for outgoing connections, so an allocation cannot collide with
/// a socket the kernel opened while nobody was looking.
const HOST_PORT_RANGE: std::ops::Range<u16> = 20000..29000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Port {
    pub id: String,
    pub service_id: String,
    /// What the process listens on inside the container.
    pub container_port: u16,
    /// The node's port when this one is published as raw TCP.
    pub host_port: Option<u16>,
    /// The hostname this port answers HTTPS on.
    pub hostname: Option<String>,
}

/// Declare a port on a service.
///
/// `publish` allocates a node port; `hostname` is stored as given and
/// must already have been checked to resolve here — this layer stores
/// what it is told, and the check belongs where there is somebody to
/// report it to.
pub async fn create(
    database: &SqliteDatabase,
    service_id: &str,
    container_port: u16,
    publish: bool,
    hostname: Option<&str>,
) -> PlatformResult<Port> {
    if container_port == 0 {
        return Err(PlatformError::Refused(
            "a port is a number between 1 and 65535".into(),
        ));
    }
    let hostname = hostname.map(normalize_hostname).filter(|h| !h.is_empty());

    let host_port = if publish {
        Some(free_host_port(database).await?)
    } else {
        None
    };

    let port = Port {
        id: format!("prt-{}", wabot::prelude::password::generate(12)),
        service_id: service_id.to_string(),
        container_port,
        host_port,
        hostname,
    };

    let row = port.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO port \
                   (\"id\", \"service_id\", \"container_port\", \"host_port\", \
                    \"hostname\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                (
                    row.id,
                    row.service_id,
                    row.container_port,
                    row.host_port,
                    row.hostname,
                    now_ms(),
                ),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| refusal(error, container_port, &port.hostname))?;

    Ok(port)
}

/// Turn the one constraint that can fail into something actionable.
///
/// Three unique indexes cover this table and "constraint failed" names
/// none of them, so the message is reconstructed from which value the
/// caller supplied.
fn refusal(
    error: wabot::sqlite::SqliteError,
    container_port: u16,
    hostname: &Option<String>,
) -> PlatformError {
    let text = error.to_string();
    if !text.contains("UNIQUE") {
        return PlatformError::Storage(error);
    }
    PlatformError::Refused(if text.contains("hostname") {
        format!(
            "{} is already used by another service",
            hostname.as_deref().unwrap_or("that hostname")
        )
    } else if text.contains("host_port") {
        "the node ran out of free ports between deciding on one and taking it — try again".into()
    } else {
        format!("this service already declares port {container_port}")
    })
}

/// The lowest free port on the node.
///
/// The unique index is what makes an allocation safe rather than this
/// check: two concurrent creates can both see the same port free, and
/// one insert loses — which is the correct outcome, and why the caller
/// gets a message telling them to try again.
async fn free_host_port(database: &SqliteDatabase) -> PlatformResult<u16> {
    let taken: Vec<i64> = database
        .read(|connection| {
            connection
                .prepare("SELECT \"host_port\" FROM port WHERE \"host_port\" IS NOT NULL")?
                .query_map([], |row| row.get(0))?
                .collect()
        })
        .await?;

    HOST_PORT_RANGE
        .map(i64::from)
        .find(|candidate| !taken.contains(candidate))
        .map(|port| port as u16)
        .ok_or_else(|| {
            PlatformError::Refused(format!(
                "every port in {}..{} is published — that is {} of them",
                HOST_PORT_RANGE.start,
                HOST_PORT_RANGE.end,
                taken.len()
            ))
        })
}

pub async fn of_service(database: &SqliteDatabase, service_id: &str) -> PlatformResult<Vec<Port>> {
    let id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"service_id\", \"container_port\", \"host_port\", \
                     \"hostname\" FROM port WHERE \"service_id\" = ?1 \
                     ORDER BY \"container_port\"",
                )?
                .query_map([id], decode)?
                .collect()
        })
        .await?)
}

/// Every port on the node, for the edge and the certificate loop.
pub async fn all(database: &SqliteDatabase) -> PlatformResult<Vec<Port>> {
    Ok(database
        .read(|connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"service_id\", \"container_port\", \"host_port\", \
                     \"hostname\" FROM port ORDER BY \"service_id\", \"container_port\"",
                )?
                .query_map([], decode)?
                .collect()
        })
        .await?)
}

pub async fn delete(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM port WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

fn decode(row: &wabot::sqlite::rusqlite::Row<'_>) -> wabot::sqlite::rusqlite::Result<Port> {
    Ok(Port {
        id: row.get(0)?,
        service_id: row.get(1)?,
        container_port: row.get::<_, i64>(2)? as u16,
        host_port: row.get::<_, Option<i64>>(3)?.map(|port| port as u16),
        hostname: row.get(4)?,
    })
}

/// A hostname as it will be compared and stored.
///
/// Lowercase, no trailing dot, no scheme, no path — all of which
/// somebody pastes, and none of which change which name is meant. The
/// edge normalizes the `Host` header the same way, and a route stored
/// differently from how it is looked up is a route that never matches.
pub fn normalize_hostname(hostname: &str) -> String {
    let hostname = hostname.trim().to_ascii_lowercase();
    let hostname = hostname
        .strip_prefix("https://")
        .or_else(|| hostname.strip_prefix("http://"))
        .unwrap_or(&hostname);
    hostname
        .split('/')
        .next()
        .unwrap_or_default()
        .trim_end_matches('.')
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = super::super::projects::create(&database, "demo")
            .await
            .expect("project");
        let service = super::super::services::create(
            &database,
            &project.id,
            "web",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        (database, service.id)
    }

    /// The common case: a port exists, and nothing outside the node
    /// can reach it.
    #[tokio::test]
    async fn a_plain_port_is_exposed_to_nobody() {
        let (database, service) = service().await;
        let port = create(&database, &service, 8080, false, None)
            .await
            .expect("created");

        assert_eq!(port.container_port, 8080);
        assert!(port.host_port.is_none() && port.hostname.is_none());
        assert_eq!(
            of_service(&database, &service).await.expect("query"),
            [port]
        );
    }

    #[tokio::test]
    async fn publishing_allocates_a_node_port() {
        let (database, service) = service().await;
        let port = create(&database, &service, 5432, true, None)
            .await
            .expect("created");

        let host_port = port.host_port.expect("allocated");
        assert!(HOST_PORT_RANGE.contains(&host_port), "{host_port}");
        assert!(port.hostname.is_none(), "publishing TCP is not HTTPS");
    }

    /// Two services publishing means two node ports, or one of them is
    /// answering for the other.
    #[tokio::test]
    async fn two_published_ports_never_share_a_node_port() {
        let (database, service) = service().await;
        let first = create(&database, &service, 5432, true, None)
            .await
            .expect("created");
        let second = create(&database, &service, 6379, true, None)
            .await
            .expect("created");

        assert_ne!(first.host_port, second.host_port);
    }

    #[tokio::test]
    async fn a_hostname_belongs_to_one_port_on_the_node() {
        let (database, service) = service().await;
        create(&database, &service, 80, false, Some("api.example.com"))
            .await
            .expect("created");

        let error = create(&database, &service, 8080, false, Some("api.example.com"))
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("already used"), "{error}");
    }

    #[tokio::test]
    async fn a_service_cannot_declare_a_port_twice() {
        let (database, service) = service().await;
        create(&database, &service, 80, false, None)
            .await
            .expect("created");

        let error = create(&database, &service, 80, false, None)
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("already declares"), "{error}");
    }

    #[tokio::test]
    async fn deleting_a_service_takes_its_ports() {
        let (database, service) = service().await;
        create(&database, &service, 80, true, Some("a.example.com"))
            .await
            .expect("created");

        super::super::services::delete(&database, &service)
            .await
            .expect("delete");
        assert!(all(&database).await.expect("query").is_empty());
    }

    /// A route stored differently from how the edge looks it up is a
    /// route that never matches, and nothing says why.
    #[test]
    fn a_hostname_is_stored_the_way_the_edge_compares_it() {
        for (given, expected) in [
            ("API.Example.COM", "api.example.com"),
            ("  api.example.com.  ", "api.example.com"),
            ("https://api.example.com/health", "api.example.com"),
            ("http://api.example.com", "api.example.com"),
        ] {
            assert_eq!(normalize_hostname(given), expected, "given {given:?}");
        }
    }

    #[tokio::test]
    async fn port_zero_is_refused() {
        let (database, service) = service().await;
        assert!(create(&database, &service, 0, false, None).await.is_err());
    }
}
