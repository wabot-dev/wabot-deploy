//! One running copy of a service, on one node.
//!
//! The service says *what* to run; a replica says *where*, and there can
//! be several — including several on one machine. See migration `0020`.
//!
//! ## What is wired
//!
//! Deploying, stopping, observing, reconciling and routing all work a
//! replica at a time. What is not here yet is the placing: every
//! replica is on this node, because the page that puts one somewhere
//! else is the phase that comes next. The `allow`s below name the
//! handful of operations that page is what calls.

use wabot::sqlite::rusqlite::{OptionalExtension, Row};
use wabot::sqlite::SqliteDatabase;

use super::{now_ms, PlatformResult};

/// One placement of a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replica {
    pub id: String,
    pub service_id: String,
    /// Which node runs it. `None` is this one — see migration `0020`.
    pub node_id: Option<String>,
    /// Its number within the service, from 1.
    pub slot: u32,
    pub address: Option<String>,
    /// The port on **this node's overlay address** that reaches this
    /// copy, for an edge running somewhere else. `None` until something
    /// outside the node needs to reach it. See migration `0022`.
    pub overlay_port: Option<u16>,
    pub last_error: Option<String>,
    /// Set when the node running it threw it out. The node that placed
    /// it stops asking.
    pub evicted_at: Option<i64>,
    /// The last octet of the address this copy keeps on its project's
    /// bridge, for as long as the copy exists.
    ///
    /// `None` is every copy that does not need one — which is all of
    /// them except a database's. See migration `0030`: a connection
    /// string is written down somewhere, so the address behind it
    /// cannot move when the container is recreated, and `host-local`
    /// hands out the lowest free one.
    pub reserved_host: Option<u8>,
}

impl Replica {
    /// Whether this copy runs on the node reading the row.
    pub fn is_here(&self) -> bool {
        self.node_id.is_none()
    }

    pub fn evicted(&self) -> bool {
        self.evicted_at.is_some()
    }

    /// The containerd container id for this copy.
    ///
    /// **Slot 1 keeps the name the service always had.** A container id
    /// is what reconciliation matches a running container against, so
    /// renaming every existing one would make an upgrade start a second
    /// copy of everything and leave the first running under a name
    /// nothing looks for any more. The suffix appears from slot 2, on
    /// containers that did not exist before this did.
    pub fn container_id(&self, project_slug: &str, service_slug: &str) -> String {
        container_id_for(project_slug, service_slug, self.slot)
    }
}

/// The same id, from a slot number rather than a row.
///
/// For a caller that has the slot and not the replica — the log stream,
/// which is handed one in a query string and must build the same id the
/// deploy did. One function so the two cannot drift: the rule that slot
/// 1 keeps the old name is the sort that gets reimplemented slightly
/// differently and then costs an afternoon.
pub fn container_id_for(project_slug: &str, service_slug: &str, slot: u32) -> String {
    match slot {
        1 => format!("{project_slug}.{service_slug}"),
        slot => format!("{project_slug}.{service_slug}.{slot}"),
    }
}

/// Bring a service to `count` replicas here, and hand them back.
///
/// Convergent, and it fills gaps rather than counting: slots are what a
/// replica is placed in, so a service that lost slot 2 gets slot 2 back
/// rather than a slot 4 nobody asked for.
pub async fn ensure_here(
    database: &SqliteDatabase,
    service_id: &str,
    count: u32,
) -> PlatformResult<Vec<Replica>> {
    let existing = of_service(database, service_id).await?;
    for slot in 1..=count {
        if existing.iter().any(|replica| replica.slot == slot) {
            continue;
        }
        place(database, service_id, None, slot).await?;
    }
    of_service(database, service_id).await
}

/// Bring the number of copies to `wanted`, adding on `node`.
///
/// Counts rather than filling `1..=wanted`, which is the difference
/// that matters after something was removed: a service left holding
/// slots 1 and 3 already *has* two copies, and filling the range would
/// put slot 2 back — undoing the removal that just happened. New ones
/// take the lowest free slot, so the numbering stays dense.
///
/// `node` is where the new ones go — `None` for here. Asked for rather
/// than assumed: putting a copy where it is wanted is one instruction,
/// and "add it here, then move it" is two, with a container started and
/// stopped on this machine in between for nothing.
pub async fn ensure_count(
    database: &SqliteDatabase,
    service_id: &str,
    wanted: u32,
    node: Option<&str>,
) -> PlatformResult<Vec<Replica>> {
    loop {
        let existing = of_service(database, service_id).await?;
        if existing.len() as u32 >= wanted {
            return Ok(existing);
        }
        let free = (1u32..)
            .find(|slot| !existing.iter().any(|replica| replica.slot == *slot))
            .expect("the naturals do not run out");
        place(database, service_id, node, free).await?;
    }
}

/// Make sure exactly these slots exist here, and hand back what runs.
///
/// **A slot number belongs to the service, not to the node.** When one
/// node places slots 2 and 3 on another, they are slots 2 and 3 there
/// too — not that node's own 1 and 2. Two reasons, and the second is
/// what phase 6 needs: the container ids match on both sides, so a
/// replica moving home is the same container; and a report about "slot
/// 3" names one thing across the whole network, with nothing to map
/// between.
///
/// Missing slots are created. Extra ones are **left alone**: taking a
/// replica away is stopping a container, which is an instruction of its
/// own and not a side effect of a list arriving without it.
pub async fn ensure_slots(
    database: &SqliteDatabase,
    service_id: &str,
    slots: &[u32],
) -> PlatformResult<Vec<Replica>> {
    let existing = of_service(database, service_id).await?;
    for slot in slots {
        if existing.iter().any(|replica| replica.slot == *slot) {
            continue;
        }
        place(database, service_id, None, *slot).await?;
    }
    of_service(database, service_id).await
}

/// Put one replica in a slot, on a node or here.
#[allow(dead_code)]
pub async fn place(
    database: &SqliteDatabase,
    service_id: &str,
    node_id: Option<&str>,
    slot: u32,
) -> PlatformResult<Replica> {
    let replica = Replica {
        id: format!("rp-{}", wabot::prelude::password::generate(12)),
        service_id: service_id.to_string(),
        node_id: node_id.map(str::to_string),
        slot,
        address: None,
        overlay_port: None,
        last_error: None,
        evicted_at: None,
        reserved_host: None,
    };

    let row = replica.clone();
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO replica \
                   (\"id\", \"service_id\", \"node_id\", \"slot\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                (row.id, row.service_id, row.node_id, row.slot, now_ms()),
            )?;
            Ok(())
        })
        .await?;

    Ok(replica)
}

/// Move a replica to another node, or bring it here.
///
/// The slot is kept, so the container id is kept, so a replica that
/// comes back to a node it ran on before is the same container rather
/// than a second one beside it.
#[allow(dead_code)]
pub async fn move_to(
    database: &SqliteDatabase,
    id: &str,
    node_id: Option<&str>,
) -> PlatformResult<()> {
    let (id, node_id) = (id.to_string(), node_id.map(str::to_string));
    database
        .write(move |connection| {
            // Moving clears where it was: the address belonged to the
            // old node's bridge, and an errand that failed there is not
            // a fact about where it is going.
            connection.execute(
                "UPDATE replica SET \"node_id\" = ?2, \"address\" = NULL, \
                 \"last_error\" = NULL, \"evicted_at\" = NULL WHERE \"id\" = ?1",
                (id, node_id),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Every replica of a service, by slot.
pub async fn of_service(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Vec<Replica>> {
    let service_id = service_id.to_string();
    Ok(database
        .read(move |connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {COLUMNS} FROM replica WHERE \"service_id\" = ?1 ORDER BY \"slot\""
            ))?;
            let replicas: wabot::sqlite::rusqlite::Result<Vec<Replica>> =
                statement.query_map([service_id], decode)?.collect();
            replicas
        })
        .await?)
}

/// Every replica this node is the one running.
///
/// What reconciliation on boot asks for: the containers that should be
/// up here, whoever placed them.
pub async fn here(database: &SqliteDatabase) -> PlatformResult<Vec<Replica>> {
    Ok(database
        .read(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {COLUMNS} FROM replica \
                 WHERE \"node_id\" IS NULL AND \"evicted_at\" IS NULL \
                 ORDER BY \"service_id\", \"slot\""
            ))?;
            let replicas: wabot::sqlite::rusqlite::Result<Vec<Replica>> =
                statement.query_map([], decode)?.collect();
            replicas
        })
        .await?)
}

/// Every replica this node placed **somewhere else**.
///
/// The other half of `here`, and what the route table needs: this node
/// is the edge for its own services' names, so a name whose service has
/// a copy on another node has to reach it from here too. Evicted ones
/// are left out — nothing is running there to reach.
pub async fn elsewhere(database: &SqliteDatabase) -> PlatformResult<Vec<Replica>> {
    Ok(database
        .read(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {COLUMNS} FROM replica \
                 WHERE \"node_id\" IS NOT NULL AND \"evicted_at\" IS NULL \
                 ORDER BY \"service_id\", \"slot\""
            ))?;
            let replicas: wabot::sqlite::rusqlite::Result<Vec<Replica>> =
                statement.query_map([], decode)?.collect();
            replicas
        })
        .await?)
}

/// The copy of a service that runs on this node, if one does.
///
/// What a page showing a service as one thing reads its address and its
/// reason from. There is exactly one until somebody places a second
/// here, and then this is the first — the page that shows them all is
/// the one that comes with placing them.
pub async fn here_for(
    database: &SqliteDatabase,
    service_id: &str,
) -> PlatformResult<Option<Replica>> {
    Ok(of_service(database, service_id)
        .await?
        .into_iter()
        .find(|replica| replica.is_here() && !replica.evicted()))
}

/// One copy of a service, by the slot it occupies.
///
/// How a report from another node lands: it names a service and a slot,
/// which is one row across the whole network because slot numbers are
/// the service's rather than the node's.
pub async fn in_slot(
    database: &SqliteDatabase,
    service_id: &str,
    slot: u32,
) -> PlatformResult<Option<Replica>> {
    Ok(of_service(database, service_id)
        .await?
        .into_iter()
        .find(|replica| replica.slot == slot))
}

#[allow(dead_code)]
pub async fn find(database: &SqliteDatabase, id: &str) -> PlatformResult<Option<Replica>> {
    let id = id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .query_row(
                    &format!("SELECT {COLUMNS} FROM replica WHERE \"id\" = ?1"),
                    [id],
                    decode,
                )
                .optional()
        })
        .await?)
}

/// Ports for reaching a replica from another node.
///
/// A range of their own, so this allocator and the one handing out
/// published host ports cannot collide by construction rather than by
/// both remembering to check the other's table.
const OVERLAY_PORT_RANGE: std::ops::Range<u16> = 30000..31000;

/// Give this copy a port on the node's overlay address, if it has none.
///
/// Convergent, and it keeps the port it already had: the number is in
/// the upstream list some edge is holding, so re-deploying a replica
/// must not move it.
pub async fn ensure_overlay_port(database: &SqliteDatabase, id: &str) -> PlatformResult<u16> {
    if let Some(existing) = find(database, id).await?.and_then(|r| r.overlay_port) {
        return Ok(existing);
    }

    let taken: Vec<u16> = database
        .read(|connection| {
            let mut statement = connection.prepare(
                "SELECT \"overlay_port\" FROM replica WHERE \"overlay_port\" IS NOT NULL",
            )?;
            let ports: wabot::sqlite::rusqlite::Result<Vec<i64>> =
                statement.query_map([], |row| row.get(0))?.collect();
            ports
        })
        .await?
        .into_iter()
        .map(|port| port as u16)
        .collect();

    let port = OVERLAY_PORT_RANGE
        .clone()
        .find(|candidate| !taken.contains(candidate))
        .ok_or_else(|| {
            super::PlatformError::Refused(
                "every overlay port on this node is in use — that is 1000 replicas \
                 reachable from elsewhere"
                    .into(),
            )
        })?;

    let (id, stored) = (id.to_string(), port as i64);
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE replica SET \"overlay_port\" = ?2 WHERE \"id\" = ?1",
                (id, stored),
            )?;
            Ok(())
        })
        .await?;
    Ok(port)
}

/// Give this copy an address on its project's bridge that will not
/// move, if it has none.
///
/// Convergent, and it keeps the one it already had — the number is in a
/// connection string somebody wrote into an application, so recreating
/// the container must not change it.
///
/// Allocated per **project**, because that is the scope of a bridge:
/// two projects each have a `10.42.x.200` and they are different
/// machines' worth of network.
///
/// **From the top down**, which is the whole of the separation from
/// `host-local`'s automatic allocation — that one counts up from the
/// bottom of the same range. Not a fence: the plugin's reservation
/// files are what actually stop a collision, and this only keeps the
/// two ends apart until a project has ~250 containers. See
/// `runtime::network::RESERVED_HOSTS`, where two tidier constructions
/// were tried and both failed on a node.
pub async fn reserve_host(
    database: &SqliteDatabase,
    project_id: &str,
    id: &str,
) -> PlatformResult<u8> {
    if let Some(existing) = find(database, id).await?.and_then(|r| r.reserved_host) {
        return Ok(existing);
    }

    let project = project_id.to_string();
    let taken: Vec<u8> = database
        .read(move |connection| {
            let mut statement = connection.prepare(
                "SELECT replica.\"reserved_host\" FROM replica \
                 JOIN service ON service.\"id\" = replica.\"service_id\" \
                 WHERE service.\"project_id\" = ?1 AND replica.\"reserved_host\" IS NOT NULL",
            )?;
            let hosts: wabot::sqlite::rusqlite::Result<Vec<i64>> =
                statement.query_map([project], |row| row.get(0))?.collect();
            hosts
        })
        .await?
        .into_iter()
        .map(|host| host as u8)
        .collect();

    // Cloned rather than iterated in place: a `const` is a value, so
    // every mention of it is a fresh temporary and `find` would be
    // advancing one nobody else can see.
    let host = crate::runtime::network::RESERVED_HOSTS
        .clone()
        .rev()
        .find(|candidate| !taken.contains(candidate))
        .ok_or_else(|| {
            super::PlatformError::Refused(
                "every reserved address in this project's subnet is in use".into(),
            )
        })?;

    let (id, stored) = (id.to_string(), i64::from(host));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE replica SET \"reserved_host\" = ?2 WHERE \"id\" = ?1",
                (id, stored),
            )?;
            Ok(())
        })
        .await?;
    Ok(host)
}

/// Where this copy's container answers, while it is up.
pub async fn set_address(
    database: &SqliteDatabase,
    id: &str,
    address: Option<&str>,
) -> PlatformResult<()> {
    let (id, address) = (id.to_string(), address.map(str::to_string));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE replica SET \"address\" = ?2 WHERE \"id\" = ?1",
                (id, address),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Record the port another node says its copy answers on.
///
/// Written by the report, not by an allocator: the number belongs to
/// the node running the replica — it came out of *its* port space — and
/// this side only remembers it so an edge can be given the upstream.
pub async fn set_overlay_port(
    database: &SqliteDatabase,
    id: &str,
    port: Option<u16>,
) -> PlatformResult<()> {
    let (id, port) = (id.to_string(), port.map(i64::from));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE replica SET \"overlay_port\" = ?2 WHERE \"id\" = ?1",
                (id, port),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Why this copy is not running, or that it is.
pub async fn set_last_error(
    database: &SqliteDatabase,
    id: &str,
    error: Option<&str>,
) -> PlatformResult<()> {
    let (id, error) = (id.to_string(), error.map(str::to_string));
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE replica SET \"last_error\" = ?2 WHERE \"id\" = ?1",
                (id, error),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// The node running it threw it out.
///
/// Recorded rather than deleted: the node that placed it has to be able
/// to see that this happened and stop asking, and a row that vanished
/// would look like one that was never placed.
pub async fn evict(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute(
                "UPDATE replica SET \"evicted_at\" = ?2, \"address\" = NULL \
                 WHERE \"id\" = ?1 AND \"evicted_at\" IS NULL",
                (id, now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn remove(database: &SqliteDatabase, id: &str) -> PlatformResult<()> {
    let id = id.to_string();
    database
        .write(move |connection| {
            connection.execute("DELETE FROM replica WHERE \"id\" = ?1", [id])?;
            Ok(())
        })
        .await?;
    Ok(())
}

const COLUMNS: &str = "\"id\", \"service_id\", \"node_id\", \"slot\", \"address\", \
                       \"last_error\", \"evicted_at\", \"overlay_port\", \"reserved_host\"";

fn decode(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Replica> {
    Ok(Replica {
        id: row.get(0)?,
        service_id: row.get(1)?,
        node_id: row.get(2)?,
        slot: row.get::<_, i64>(3)? as u32,
        address: row.get(4)?,
        last_error: row.get(5)?,
        evicted_at: row.get(6)?,
        overlay_port: row.get::<_, Option<i64>>(7)?.map(|port| port as u16),
        reserved_host: row.get::<_, Option<i64>>(8)?.map(|host| host as u8),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::{projects, services};

    async fn service() -> (SqliteDatabase, String) {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = projects::create(&database, "demo").await.expect("project");
        let service = services::create(&database, &project.id, "web", "alpine:3.23", &[])
            .await
            .expect("service");
        (database, service.id)
    }

    /// The rule an upgrade depends on. A container id is what
    /// reconciliation matches a running container against, so renaming
    /// slot 1 would start a second copy of everything already deployed
    /// and orphan the first under a name nothing looks for.
    #[test]
    fn the_first_replica_keeps_the_name_the_service_always_had() {
        let replica = |slot| Replica {
            id: "rp-1".into(),
            service_id: "svc-1".into(),
            node_id: None,
            slot,
            address: None,
            overlay_port: None,
            last_error: None,
            evicted_at: None,
            reserved_host: None,
        };

        assert_eq!(replica(1).container_id("demo", "web"), "demo.web");
        assert_eq!(replica(2).container_id("demo", "web"), "demo.web.2");
        assert_eq!(replica(17).container_id("demo", "web"), "demo.web.17");
    }

    /// A service is at least one replica. Without that a new service
    /// would be a description of something with nowhere to run, and
    /// reconciliation — which asks about replicas — would never start
    /// it. Services made before this existed were given theirs by
    /// migration `0020`.
    #[tokio::test]
    async fn a_new_service_is_one_replica_here() {
        let (database, service) = service().await;

        let replicas = of_service(&database, &service).await.expect("replicas");
        assert_eq!(replicas.len(), 1, "{replicas:?}");
        assert_eq!(replicas[0].slot, 1);
        assert!(replicas[0].is_here());
    }

    /// The number is in an upstream list some edge is holding, so
    /// re-deploying a replica must not move it.
    #[tokio::test]
    async fn a_copy_keeps_the_overlay_port_it_was_given() {
        let (database, service) = service().await;
        let replicas = of_service(&database, &service).await.expect("replicas");

        let first = ensure_overlay_port(&database, &replicas[0].id)
            .await
            .expect("port");
        let again = ensure_overlay_port(&database, &replicas[0].id)
            .await
            .expect("port");
        assert_eq!(first, again);
        assert!(OVERLAY_PORT_RANGE.contains(&first), "{first}");
    }

    /// Two copies on one machine cannot share a host port, and they are
    /// exactly the case this exists for — an edge needs one upstream
    /// per replica so a node running two takes twice the requests.
    #[tokio::test]
    async fn two_copies_on_one_node_get_different_ports() {
        let (database, service) = service().await;
        let replicas = ensure_here(&database, &service, 2).await.expect("two");

        let one = ensure_overlay_port(&database, &replicas[0].id)
            .await
            .expect("port");
        let two = ensure_overlay_port(&database, &replicas[1].id)
            .await
            .expect("port");
        assert_ne!(one, two, "both copies answer on one port");
    }

    /// The overlay range and the published-host-port range are separate
    /// so the two allocators cannot collide by construction, rather
    /// than by both remembering to check the other's table.
    #[test]
    fn the_two_port_spaces_do_not_overlap() {
        let published = super::super::ports::HOST_PORT_RANGE;
        assert!(
            OVERLAY_PORT_RANGE.start >= published.end || OVERLAY_PORT_RANGE.end <= published.start,
            "an overlay port could be handed out twice"
        );
    }

    /// A slot number belongs to the service, not to the node it lands
    /// on. A node given slots 2 and 3 runs slots 2 and 3 — so the
    /// container ids match at both ends, and a report about "slot 3"
    /// names one thing across the network with nothing to map between.
    #[tokio::test]
    async fn a_node_told_to_run_slots_two_and_three_runs_those() {
        let (database, service) = service().await;

        let placed = ensure_slots(&database, &service, &[2, 3])
            .await
            .expect("slots");
        assert_eq!(
            placed.iter().map(|r| r.slot).collect::<Vec<_>>(),
            vec![1, 2, 3],
            "slot 1 came from `create`; 2 and 3 were named"
        );

        let two = placed.iter().find(|r| r.slot == 2).expect("two");
        assert_eq!(two.container_id("demo", "web"), "demo.web.2");
    }

    /// A list arriving without a slot is not an instruction to stop it.
    /// Taking a replica away is stopping a container, which is its own
    /// errand rather than a side effect of an omission.
    #[tokio::test]
    async fn a_slot_left_out_is_left_alone() {
        let (database, service) = service().await;
        ensure_slots(&database, &service, &[1, 2])
            .await
            .expect("slots");

        let after = ensure_slots(&database, &service, &[1]).await.expect("one");
        assert_eq!(after.len(), 2, "an omission stopped a container");
    }

    /// Slots are positions, not a count. A service that lost its second
    /// replica gets its second replica back, on the container id that
    /// copy always had — not a third slot nobody asked for.
    #[tokio::test]
    async fn a_freed_slot_is_filled_again() {
        let (database, service) = service().await;
        ensure_here(&database, &service, 3).await.expect("three");

        let second = of_service(&database, &service)
            .await
            .expect("replicas")
            .into_iter()
            .find(|replica| replica.slot == 2)
            .expect("a second");
        remove(&database, &second.id).await.expect("remove");

        let refilled = ensure_here(&database, &service, 3).await.expect("three");
        assert_eq!(
            refilled.iter().map(|r| r.slot).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    /// Asking for the same number twice is asking for nothing.
    #[tokio::test]
    async fn ensuring_the_same_count_changes_nothing() {
        let (database, service) = service().await;
        let first = ensure_here(&database, &service, 2).await.expect("two");
        let again = ensure_here(&database, &service, 2).await.expect("two");
        assert_eq!(first, again);
    }

    /// A replica that comes back to a node it ran on before is the same
    /// container, because the slot — and so the id — came with it.
    #[tokio::test]
    async fn moving_keeps_the_slot_and_forgets_where_it_was() {
        let (database, service) = service().await;
        let replicas = ensure_here(&database, &service, 2).await.expect("two");
        let second = replicas.into_iter().find(|r| r.slot == 2).expect("second");
        set_address(&database, &second.id, Some("10.42.1.7"))
            .await
            .expect("address");

        move_to(&database, &second.id, Some("nd-elsewhere"))
            .await
            .expect("move");

        let moved = find(&database, &second.id)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(moved.slot, 2, "the container id would have changed");
        assert!(!moved.is_here());
        assert_eq!(moved.address, None, "it kept an address on another bridge");
    }

    /// Reconciliation on boot asks what should be up *here*. A replica
    /// placed elsewhere is not this node's to start, and one that was
    /// thrown out is not either.
    #[tokio::test]
    async fn only_what_runs_here_and_was_not_evicted_is_this_nodes_to_start() {
        let (database, service) = service().await;
        let replicas = ensure_here(&database, &service, 3).await.expect("three");

        move_to(&database, &replicas[1].id, Some("nd-elsewhere"))
            .await
            .expect("move");
        evict(&database, &replicas[2].id).await.expect("evict");

        let mine = here(&database).await.expect("here");
        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].slot, 1);
    }

    /// Eviction is recorded, not deleted: the node that placed it has
    /// to see that this happened, and a row that vanished would look
    /// like one that was never placed.
    #[tokio::test]
    async fn an_eviction_is_a_fact_that_stays_readable() {
        let (database, service) = service().await;
        let replicas = ensure_here(&database, &service, 1).await.expect("one");
        set_address(&database, &replicas[0].id, Some("10.42.1.7"))
            .await
            .expect("address");

        evict(&database, &replicas[0].id).await.expect("evict");

        let evicted = find(&database, &replicas[0].id)
            .await
            .expect("query")
            .expect("still there");
        assert!(evicted.evicted());
        assert_eq!(evicted.address, None, "it is not running anywhere");
    }

    /// Deleting a service takes its placements with it — a replica of
    /// nothing is a container nobody would ever look for again.
    #[tokio::test]
    async fn replicas_go_with_the_service() {
        let (database, service) = service().await;
        ensure_here(&database, &service, 2).await.expect("two");

        services::delete(&database, &service).await.expect("delete");
        assert!(of_service(&database, &service)
            .await
            .expect("replicas")
            .is_empty());
    }
}
