//! Tell the nodes holding a copy what to run, and take a copy off one
//! that should no longer hold it.

use wabot::sqlite::SqliteDatabase;

use crate::entities::database::Database;
use crate::entities::service::Service;
use crate::platform::{PlatformError, PlatformResult};
use crate::repositories::databases::of_service;
/// Tell every node holding a standby of this database what to hold.
///
/// **Recomputed, not emitted.** The port the primary answers on comes
/// out of this node's own port space and is assigned when the primary
/// deploys, so an errand written at the moment somebody clicked would
/// carry an address that did not exist yet. This is called after
/// anything that could have settled it, and `queue_if_changed` makes
/// calling it often free.
///
/// Nothing is queued while the primary has no address: a standby told to
/// follow nowhere would come up as a primary of its own, holding a copy
/// of somebody's data and accepting writes into it.
pub async fn dispatch(
    database: &SqliteDatabase,
    service: &Service,
    primary: Option<(String, u16)>,
    domain: Option<String>,
    // Nodes to tell **even though they hold nothing now**.
    //
    // The list below is built from the rows as they are, so a node that
    // just lost its last copy is not in it — and it is the one that most
    // needs telling. Its container goes on running, holding a volume and
    // following a primary, with nothing on either side pointing at it.
    //
    // Found on the test node: a database reduced from three copies to
    // one left `db-test.orders.3` running on the other machine for ever.
    // The `host` path solved this with the same set; this one had the
    // same hole and no `slots: []` ever reached the far side.
    also_tell: &std::collections::BTreeSet<String>,
) -> PlatformResult<usize> {
    let Some(row) = of_service(database, &service.id).await? else {
        return Ok(0);
    };
    // A database this node was told to hold is not this node's to place
    // anywhere. Its own authority decides.
    if !service.is_ours() || row.primary_endpoint.is_some() {
        return Ok(0);
    }
    let Some((host, port)) = primary else {
        return Ok(0);
    };

    let project = crate::repositories::projects::find(database, &service.project_id)
        .await?
        .ok_or_else(|| PlatformError::Refused("no project for this database".into()))?;
    let running = crate::repositories::services::find(database, &service.id)
        .await?
        .map_or(service.desired_state, |row| row.desired_state);
    let placements = crate::repositories::replicas::of_service(database, &service.id).await?;

    // Which node holds which slots. A node holding two copies is told
    // about both in one instruction, because the instruction is the
    // whole of what that node runs for this database — the same rule a
    // `host` errand follows, and what lets a copy be taken away.
    let mut by_node: std::collections::BTreeMap<String, Vec<u32>> = Default::default();
    // An empty list for a node that has just lost everything, so the
    // instruction says "none of this is yours" rather than never arriving.
    for node_id in also_tell {
        by_node.entry(node_id.clone()).or_default();
    }
    for replica in placements.iter().filter(|r| !r.evicted()) {
        let Some(node_id) = &replica.node_id else {
            continue;
        };
        if replica.slot == row.primary_slot {
            continue;
        }
        by_node
            .entry(node_id.clone())
            .or_default()
            .push(replica.slot);
    }

    // The same function the placement form offers nodes with, so what
    // was offerable and what is dispatchable cannot drift apart — they
    // did, and this loop is where it showed.
    let allowed: Vec<String> =
        crate::network::candidates_for(database, service.kind.placement_capability())
            .await
            .map_err(|error| PlatformError::Refused(error.to_string()))?
            .into_iter()
            .map(|node| node.id)
            .collect();
    let mut sent = 0;

    for (node_id, slots) in by_node {
        // A node that never agreed to keep data is not somewhere a copy
        // can go, and queueing against it produces an errand it refuses
        // for ever while this side says the copy is placed.
        //
        // **Said out loud on the row, not only in the journal.** This
        // skipped in silence, so a copy placed on a node without `store`
        // read as placed on the page that placed it and was never sent
        // anywhere — a failure that only reached the journal is a failure
        // nobody sees. The picker refuses the node now, and this is the
        // belt for a grant withdrawn after the row was written.
        if !allowed.contains(&node_id) {
            tracing::debug!(node = %node_id, "skipped: it has not agreed to keep data here");
            for slot in &slots {
                let Some(replica) = placements.iter().find(|replica| {
                    replica.slot == *slot && replica.node_id.as_deref() == Some(&*node_id)
                }) else {
                    continue;
                };
                let _ = crate::repositories::replicas::set_last_error(
                    database,
                    &replica.id,
                    Some("that node has not agreed to keep data for this one"),
                )
                .await;
            }
            continue;
        }

        let payload = instruction(
            &project.name,
            service,
            &row,
            &format!("{host}:{port}"),
            &domain,
            slots,
            running == crate::entities::service::DesiredState::Running,
        )?;

        let queued = crate::network::errand::queue_if_changed(
            database,
            &node_id,
            crate::network::errand::Kind::Database,
            // One database, so a node holding standbys of two is told
            // about each without one overwriting the other's history.
            &format!("database:{}", service.id),
            &payload,
        )
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;

        if queued.is_some() {
            tracing::info!(service = %service.slug, node = %node_id, "asked a node to hold a standby");
            sent += 1;
        }
    }
    Ok(sent)
}
/// One instruction, for one node and one set of slots.
///
/// Shared by [`dispatch`] and [`withdraw`] because the difference between
/// them is two fields: letting go is the same description of the same
/// database with nothing left to hold. Two builders would drift, and the
/// field that went missing would be the one nobody reads until a copy
/// behaves as if it were never told.
fn instruction(
    project: &str,
    service: &Service,
    row: &Database,
    primary: &str,
    domain: &Option<String>,
    slots: Vec<u32>,
    running: bool,
) -> PlatformResult<serde_json::Value> {
    serde_json::to_value(crate::network::errand::Standby {
        project: project.to_string(),
        service: service.name.clone(),
        image: service.image.clone(),
        registry: crate::values::image_reference::ImageReference::parse(&service.image)
            .ok()
            .and_then(|r| r.host())
            .unwrap_or_default(),
        // No credential: a database's image comes from Docker Hub, which
        // serves anybody. `send_there` explains what happens when one is
        // sent to a registry that is not this node's.
        username: None,
        secret: None,
        memory_limit: service
            .memory_limit
            .unwrap_or(crate::policies::presets::SMALLEST),
        engine: row.engine.as_str().to_string(),
        version: row.version.clone(),
        database_name: row.database_name.clone(),
        admin_user: row.admin_user.clone(),
        admin_password: row.admin_password.clone(),
        replication_user: row.replication_user.clone(),
        replication_password: row.replication_password.clone(),
        primary: primary.to_string(),
        slots,
        primary_slot: row.primary_slot,
        // The owner's, because the copy has to answer to the *database's*
        // qualified name rather than to one built from the domain of
        // whichever machine is holding it.
        qualified_domain: domain.clone(),
        running,
    })
    .map_err(|error| PlatformError::Refused(error.to_string()))
}
/// Tell every node holding a copy of this database to let it go.
///
/// `slots: []`, which the holding node reads as "this is not yours to
/// keep any more": it stops the copy and deletes its own rows. Safe for a
/// standby in a way it would never be for a primary — every row of it is
/// on the node that owns it — and it is the one instruction a deletion
/// can mean.
///
/// **It has to leave before the rows it is built from do**, and it is the
/// one instruction in this system with no second chance: every other one
/// is recomputed at the next boot from rows that are still here, and after
/// a deletion there are none. A copy this never reached goes on running
/// with nobody left to contradict it.
///
/// The primary's endpoint is what the caller knows, falling back to what
/// the row was told: the far node writes it and deletes the row three
/// lines later, so what matters is that the payload is the shape it can
/// read.
pub async fn withdraw(
    database: &SqliteDatabase,
    service: &Service,
    primary: Option<(String, u16)>,
    domain: Option<String>,
) -> PlatformResult<usize> {
    let Some(row) = of_service(database, &service.id).await? else {
        return Ok(0);
    };
    // A database this node was told to hold is not this node's to give
    // away. Its own authority decides, and this node's console evicts.
    if !service.is_ours() || row.primary_endpoint.is_some() {
        return Ok(0);
    }
    let project = crate::repositories::projects::find(database, &service.project_id)
        .await?
        .ok_or_else(|| PlatformError::Refused("no project for this database".into()))?;
    let endpoint = primary
        .map(|(host, port)| format!("{host}:{port}"))
        .or_else(|| row.primary_endpoint.clone())
        .unwrap_or_default();

    let payload = instruction(
        &project.name,
        service,
        &row,
        &endpoint,
        &domain,
        Vec::new(),
        false,
    )?;

    let mut sent = 0;
    for node_id in crate::repositories::replicas::of_service(database, &service.id)
        .await?
        .into_iter()
        .filter(|replica| !replica.evicted())
        .filter_map(|replica| replica.node_id)
        .collect::<std::collections::BTreeSet<_>>()
    {
        // Queued outright rather than only when it differs. The comparison
        // exists so a pass that runs every boot stays quiet, and this runs
        // once, on the way out — where the risk worth avoiding is the
        // message not being sent at all.
        crate::network::errand::queue(
            database,
            &node_id,
            crate::network::errand::Kind::Database,
            &payload,
        )
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;
        tracing::info!(service = %service.slug, node = %node_id, "asked a node to let a copy go");
        sent += 1;
    }
    Ok(sent)
}
