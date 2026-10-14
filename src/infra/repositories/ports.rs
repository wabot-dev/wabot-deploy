//! Every `SELECT`, `INSERT` and `UPDATE` a [`Port`] needs.

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::port::Port;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::domain::values::hostname::Hostname;
use crate::infra::clock::now_ms;

/// Where published ports are allocated from.
///
/// Above the registered range and below the ephemeral one Linux hands
/// out for outgoing connections, so an allocation cannot collide with a
/// socket the kernel opened while nobody was looking.
///
/// `replicas::OVERLAY_PORT_RANGE` is deliberately disjoint from this, so
/// the two allocators cannot hand out the same number without either of
/// them consulting the other's table.
pub(crate) const HOST_PORT_RANGE: std::ops::Range<u16> = 20000..29000;

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
    // Which nodes answer for the hostname. **Empty is "whoever this node
    // is"**, which is what every caller with a name of its own means — a
    // database's name is a subdomain of this node's domain. A caller that
    // asked DNS where the name points passes the answer, because the
    // nodes it reaches are the only ones that can serve it.
    served_by: &[String],
) -> PlatformResult<Port> {
    if container_port == 0 {
        return Err(PlatformError::Refused(
            "a port is a number between 1 and 65535".into(),
        ));
    }
    let hostname = hostname
        .map(Hostname::parse)
        .filter(|name| !name.is_empty())
        .map(|name| name.as_str().to_string());

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

    if let Some(hostname) = port.hostname.as_deref() {
        serve_from(database, service_id, hostname, served_by).await?;
    }
    Ok(port)
}

/// Answer for a new name here, if this node can.
///
/// Being an edge is a row, including for the node that owns the service
/// — see migration `0024`. A name arrives with nobody answering for it,
/// and a name nobody answers for is a name that resolves to nothing, so
/// the node it was declared on takes it: that is what somebody typing a
/// hostname here meant, and what a node on its own has always done.
///
/// It lives here rather than in the console because the trap is
/// otherwise silent. A caller that made a port and did not know to write
/// the row would get a hostname stored, shown on the page, and served by
/// no one — which is exactly what the first version of this did, and a
/// routing test caught it only because it happened to assert the route.
///
/// Skipped on a node with no address the world can dial. Nothing is
/// wrong: it simply cannot be the one answering, and the picker on the
/// service page is where somebody chooses who does.
/// Who answers for this name, written here rather than by the caller.
///
/// A port made with a hostname and no edge row is a name stored, shown,
/// and served by nobody — which is what the first version did, caught
/// only because a routing test asserted the route. So this function
/// exists to be impossible to forget.
///
/// `nodes` empty means this one, when it can be an edge at all. Anything
/// else is a set somebody worked out — for the console, by asking DNS
/// where the name points, because **the nodes a name reaches are the only
/// ones that can serve it**. A row naming a node the name does not point
/// at is an errand that node will carry out for traffic that never
/// arrives.
async fn serve_from(
    database: &SqliteDatabase,
    service_id: &str,
    hostname: &str,
    nodes: &[String],
) -> PlatformResult<()> {
    let mut serving = crate::infra::repositories::edges::nodes_for(database, hostname).await?;
    let wanted: Vec<String> = match nodes.is_empty() {
        true => crate::infra::network::me(database)
            .await
            .ok()
            .flatten()
            .filter(|me| me.may_be_edge())
            .map(|me| vec![me.id])
            .unwrap_or_default(),
        false => nodes.to_vec(),
    };

    let mut changed = false;
    for node in wanted {
        if !serving.contains(&node) {
            serving.push(node);
            changed = true;
        }
    }
    if changed {
        crate::infra::repositories::edges::set(database, service_id, hostname, &serving).await?;
    }
    Ok(())
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

/// Give this port a name, or take its name away.
///
/// The uniqueness is the index's, not a query's: two operators naming one
/// hostname in the same second would both pass a check and one would win
/// the write. So the constraint refuses it and the message is built from
/// what the caller supplied — the same path `create` takes.
pub async fn set_hostname(
    database: &SqliteDatabase,
    id: &str,
    hostname: Option<&str>,
) -> PlatformResult<()> {
    // What answers for the old name, before the name changes.
    //
    // **Who answers has to move with it.** `service_edge` is keyed on
    // the hostname, so renaming the port left the rows pointing at a
    // name nothing has any more — and that row is what
    // `acme::wanted_names` reads. A renamed database therefore went on
    // ordering a certificate for the name nobody dials and never
    // ordered one for the name everybody does, which is exactly the
    // failure `docs/naming.md` is about: a certificate current, from the
    // right authority, and wrong.
    //
    // Here rather than in the console, for the reason `create` already
    // gives about writing the row itself: a caller that renames a port
    // and does not know to move this gets a name stored, shown, and
    // served by nobody.
    let port = all(database).await?.into_iter().find(|port| port.id == id);
    let was = port.as_ref().and_then(|port| port.hostname.clone());
    let service_id = port.map(|port| port.service_id);
    let serving = match &was {
        Some(was) => crate::infra::repositories::edges::nodes_for(database, was).await?,
        None => Vec::new(),
    };

    let (owned, name) = (id.to_string(), hostname.map(str::to_string));
    let refused = name.clone();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE port SET \"hostname\" = ?2 WHERE \"id\" = ?1",
                (owned, name),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| refusal(error, 0, &refused))?;

    // The old name stops being answered for whatever happens next: a
    // rename that left it would be a certificate renewed twice a day for
    // a name nothing serves.
    if let Some(was) = &was {
        if Some(was.as_str()) != hostname {
            if let Some(service_id) = &service_id {
                crate::infra::repositories::edges::set(database, service_id, was, &[]).await?;
            }
        }
    }
    // And the new one is answered for by whoever answered for the old,
    // which is what makes a rename a rename rather than a reset. A name
    // that had nobody — a port that never had one — falls back to this
    // node, the same default `create` writes.
    if let (Some(hostname), Some(service_id)) = (hostname, &service_id) {
        match serving.is_empty() {
            true => serve_from(database, service_id, hostname, &[]).await?,
            false => {
                crate::infra::repositories::edges::set(database, service_id, hostname, &serving)
                    .await?;
            }
        }
    }
    Ok(())
}

/// Publish this port on a free one of the node's, or stop publishing it.
///
/// **The number is the node's to choose, not the operator's.** Two
/// databases on one machine cannot share a host port, and a form that
/// asked for one would be asking somebody to remember what every other
/// service on the node already took. `free_host_port` picks the lowest
/// free one out of `HOST_PORT_RANGE`, and the unique index is what makes
/// that safe rather than the lookup — two concurrent publishes can both
/// see the same number free, and the loser is told to try again.
///
/// Idempotent: a port already published keeps the number it has. An
/// allocation that moved would break every client holding the old one, to
/// no purpose.
pub async fn set_published(
    database: &SqliteDatabase,
    id: &str,
    publish: bool,
) -> PlatformResult<Option<u16>> {
    let existing = all(database)
        .await?
        .into_iter()
        .find(|port| port.id == id)
        .ok_or_else(|| PlatformError::Refused("no such port".into()))?;

    let host_port = match (publish, existing.host_port) {
        (true, Some(held)) => return Ok(Some(held)),
        (true, None) => Some(free_host_port(database).await?),
        (false, None) => return Ok(None),
        (false, Some(_)) => None,
    };

    let (id, stored) = (id.to_string(), host_port.map(i64::from));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE port SET \"host_port\" = ?2 WHERE \"id\" = ?1",
                (id, stored),
            )?;
            Ok(())
        })
        .await
        .map_err(|error| refusal(error, existing.container_port, &None))?;
    Ok(host_port)
}

pub async fn delete(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            // Who answers for the name goes with the last port that has it,
            // the same rule `set_hostname` follows for a rename. Left
            // behind, the row kept `acme::wanted_names` ordering a
            // certificate for a name nothing served. Found on a VM node.
            connection.execute(
                "DELETE FROM service_edge WHERE EXISTS ( \
                   SELECT 1 FROM port AS gone \
                   WHERE gone.\"id\" = ?1 \
                     AND gone.\"service_id\" = service_edge.\"service_id\" \
                     AND gone.\"hostname\" = service_edge.\"hostname\" \
                     AND NOT EXISTS ( \
                       SELECT 1 FROM port AS kept \
                       WHERE kept.\"service_id\" = gone.\"service_id\" \
                         AND kept.\"hostname\" = gone.\"hostname\" \
                         AND kept.\"id\" != gone.\"id\"))",
                [&id],
            )?;
            connection.execute("DELETE FROM port WHERE \"id\" = ?1", [&id])?;
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

#[cfg(test)]
mod tests {
    /// **Who answers for a name has to move with the name.**
    ///
    /// `service_edge` is keyed on the hostname, so renaming a port left
    /// its rows pointing at a name nothing has any more — and that row
    /// is what `acme::wanted_names` reads. A renamed database went on
    /// ordering a certificate for the name nobody dials and never
    /// ordered one for the name everybody does: a certificate current,
    /// from the right authority, and wrong, which is what
    /// `docs/naming.md` exists about.
    ///
    /// Found by a test written about something else — the route a
    /// database should never have had — which is the second time in this
    /// file that the setup for one claim turned out to be the claim.
    #[tokio::test]
    async fn renaming_a_port_carries_who_answers_for_it() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::network::ensure_self(&database, &crate::infra::config::Config::default())
            .await
            .expect("self");
        let mut me = crate::infra::network::me(&database)
            .await
            .expect("query")
            .expect("there");
        me.kind = crate::infra::network::Kind::Public;
        me.endpoint = Some("example.com:443".into());
        crate::infra::network::save(&database, &me)
            .await
            .expect("saved");

        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "api",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        let port = create(
            &database,
            &service.id,
            80,
            false,
            Some("old.example.com"),
            &[],
        )
        .await
        .expect("port");

        assert_eq!(
            crate::infra::repositories::edges::nodes_for(&database, "old.example.com")
                .await
                .expect("query"),
            vec![me.id.clone()],
            "creating with a name makes this node answer for it"
        );

        set_hostname(&database, &port.id, Some("new.example.com"))
            .await
            .expect("renamed");

        assert_eq!(
            crate::infra::repositories::edges::nodes_for(&database, "new.example.com")
                .await
                .expect("query"),
            vec![me.id],
            "and the rename carries that over"
        );
        assert!(
            crate::infra::repositories::edges::nodes_for(&database, "old.example.com")
                .await
                .expect("query")
                .is_empty(),
            "the old name stops being answered for, or it is renewed for ever"
        );

        // And deleting the port takes it the rest of the way.
        delete(&database, &port.id).await.expect("deleted");
        assert!(
            crate::infra::repositories::edges::nodes_for(&database, "new.example.com")
                .await
                .expect("query")
                .is_empty(),
            "a deleted port's name is still answered for, so it is still ordered"
        );
    }

    /// The node picks the number, and it does not move.
    ///
    /// Two databases on one machine cannot share a host port, so asking an
    /// operator for one would be asking them to remember what everything
    /// else on the node already took. And a port that moved on the next
    /// publish would break every client holding the old one — which is why
    /// publishing something already published is a no-op rather than a
    /// fresh allocation.
    #[tokio::test]
    async fn publishing_picks_a_free_port_and_keeps_it() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "shared")
            .await
            .expect("project");
        let mut ports = Vec::new();
        for name in ["one", "two"] {
            let service = crate::infra::repositories::services::create(
                &database,
                &project.id,
                name,
                "registry.example/db@sha256:abc",
                &[],
            )
            .await
            .expect("service");
            ports.push(
                create(&database, &service.id, 5432, false, None, &[])
                    .await
                    .expect("port"),
            );
        }

        let first = set_published(&database, &ports[0].id, true)
            .await
            .expect("published")
            .expect("a number");
        let second = set_published(&database, &ports[1].id, true)
            .await
            .expect("published")
            .expect("a number");
        assert_ne!(first, second, "two databases cannot share one host port");
        assert!(HOST_PORT_RANGE.contains(&first) && HOST_PORT_RANGE.contains(&second));

        assert_eq!(
            set_published(&database, &ports[0].id, true)
                .await
                .expect("again"),
            Some(first),
            "a port already published keeps the number clients are holding"
        );

        assert_eq!(
            set_published(&database, &ports[0].id, false)
                .await
                .expect("closed"),
            None
        );
        // And the number goes back into the pool rather than being burnt.
        assert_eq!(
            set_published(&database, &ports[0].id, true)
                .await
                .expect("republished"),
            Some(first)
        );
    }

    use super::*;

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
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
        let port = create(&database, &service, 8080, false, None, &[])
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
        let port = create(&database, &service, 5432, true, None, &[])
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
        let first = create(&database, &service, 5432, true, None, &[])
            .await
            .expect("created");
        let second = create(&database, &service, 6379, true, None, &[])
            .await
            .expect("created");

        assert_ne!(first.host_port, second.host_port);
    }

    /// A name arrives with nobody answering for it, and a name nobody
    /// answers for resolves to nothing. The node it was declared on
    /// takes it — which is what a node on its own has always done, and
    /// now says so in a row that can be unticked.
    #[tokio::test]
    async fn a_new_hostname_is_served_from_the_node_it_was_declared_on() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::node::settings::set_domain(&database, Some("node.example"))
            .await
            .expect("domain");
        let me =
            crate::infra::network::ensure_self(&database, &crate::infra::config::Config::default())
                .await
                .expect("self");
        let project = crate::infra::repositories::projects::create(&database, "demo")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "web",
            "docker.io/library/alpine:3.23",
            &[],
        )
        .await
        .expect("service");

        create(
            &database,
            &service.id,
            80,
            false,
            Some("api.example.com"),
            &[],
        )
        .await
        .expect("created");

        assert_eq!(
            crate::infra::repositories::edges::nodes_for(&database, "api.example.com")
                .await
                .expect("query"),
            vec![me.id]
        );
    }

    /// A private node can own projects and services perfectly well; what
    /// it cannot do is answer for their names. Writing the row anyway
    /// would have the console showing it doing something it has no
    /// address to do.
    #[tokio::test]
    async fn a_node_with_no_public_address_does_not_take_the_name() {
        let (database, service) = service().await;

        create(&database, &service, 80, false, Some("api.example.com"), &[])
            .await
            .expect("created");

        assert!(
            crate::infra::repositories::edges::nodes_for(&database, "api.example.com")
                .await
                .expect("query")
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_hostname_belongs_to_one_port_on_the_node() {
        let (database, service) = service().await;
        create(&database, &service, 80, false, Some("api.example.com"), &[])
            .await
            .expect("created");

        let error = create(
            &database,
            &service,
            8080,
            false,
            Some("api.example.com"),
            &[],
        )
        .await
        .expect_err("refused");
        assert!(error.to_string().contains("already used"), "{error}");
    }

    #[tokio::test]
    async fn a_service_cannot_declare_a_port_twice() {
        let (database, service) = service().await;
        create(&database, &service, 80, false, None, &[])
            .await
            .expect("created");

        let error = create(&database, &service, 80, false, None, &[])
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("already declares"), "{error}");
    }

    #[tokio::test]
    async fn deleting_a_service_takes_its_ports() {
        let (database, service) = service().await;
        create(&database, &service, 80, true, Some("a.example.com"), &[])
            .await
            .expect("created");

        crate::infra::repositories::services::delete(&database, &service)
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
            assert_eq!(
                Hostname::parse(given).as_str().to_string(),
                expected,
                "given {given:?}"
            );
        }
    }

    #[tokio::test]
    async fn port_zero_is_refused() {
        let (database, service) = service().await;
        assert!(create(&database, &service, 0, false, None, &[])
            .await
            .is_err());
    }
}
