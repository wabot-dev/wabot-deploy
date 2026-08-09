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
    pub last_error: Option<String>,
    /// Set when the node running it threw it out. The node that placed
    /// it stops asking.
    pub evicted_at: Option<i64>,
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
        match self.slot {
            1 => format!("{project_slug}.{service_slug}"),
            slot => format!("{project_slug}.{service_slug}.{slot}"),
        }
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

/// Bring the number of copies to `wanted`, adding here.
///
/// Counts rather than filling `1..=wanted`, which is the difference
/// that matters after something was removed: a service left holding
/// slots 1 and 3 already *has* two copies, and filling the range would
/// put slot 2 back — undoing the removal that just happened. New ones
/// take the lowest free slot, so the numbering stays dense.
pub async fn ensure_count_here(
    database: &SqliteDatabase,
    service_id: &str,
    wanted: u32,
) -> PlatformResult<Vec<Replica>> {
    loop {
        let existing = of_service(database, service_id).await?;
        if existing.len() as u32 >= wanted {
            return Ok(existing);
        }
        let free = (1u32..)
            .find(|slot| !existing.iter().any(|replica| replica.slot == *slot))
            .expect("the naturals do not run out");
        place(database, service_id, None, free).await?;
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
        last_error: None,
        evicted_at: None,
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
#[allow(dead_code)]
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
                       \"last_error\", \"evicted_at\"";

fn decode(row: &Row<'_>) -> wabot::sqlite::rusqlite::Result<Replica> {
    Ok(Replica {
        id: row.get(0)?,
        service_id: row.get(1)?,
        node_id: row.get(2)?,
        slot: row.get::<_, i64>(3)? as u32,
        address: row.get(4)?,
        last_error: row.get(5)?,
        evicted_at: row.get(6)?,
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
            last_error: None,
            evicted_at: None,
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
