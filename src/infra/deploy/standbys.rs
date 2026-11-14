//! Tell the nodes holding a copy what to run, and take a copy off one
//! that should no longer hold it.

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::database::Database;
use crate::domain::entities::service::Service;
use crate::domain::errors::{PlatformError, PlatformResult};
use crate::infra::repositories::databases::of_service;
/// Tell every other node holding a copy of this database what to hold.
///
/// **Recomputed, not emitted.** The port the primary answers on is
/// assigned when it deploys — here, or on the node holding it, which
/// sends it home on a report — so an errand written at the moment
/// somebody clicked would carry an address that did not exist yet. This
/// is called after anything that could have settled it, and
/// `queue_if_changed` makes calling it often free.
///
/// The node holding the primary is told at once: it needs no address to
/// start. A node holding only standbys is told nothing while the primary
/// has no address: a standby told to follow nowhere would come up as a
/// primary of its own, holding a copy of somebody's data and accepting
/// writes into it.
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

    let project = crate::infra::repositories::projects::find(database, &service.project_id)
        .await?
        .ok_or_else(|| PlatformError::Refused("no project for this database".into()))?;
    let running = crate::infra::repositories::services::find(database, &service.id)
        .await?
        .map_or(service.desired_state, |row| row.desired_state);
    let placements =
        crate::infra::repositories::replicas::of_service(database, &service.id).await?;

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
        by_node
            .entry(node_id.clone())
            .or_default()
            .push(replica.slot);
    }

    let primary_node = placements
        .iter()
        .find(|replica| replica.slot == row.primary_slot)
        .and_then(|replica| replica.node_id.clone());
    let followers = followers(database, &row, &placements, primary_node.as_deref()).await?;
    let name = crate::infra::deploy::qualified_database_name(database, &project, service).await;
    let published = crate::infra::repositories::ports::of_service(database, &service.id)
        .await?
        .iter()
        .any(|port| port.host_port.is_some());

    // The same function the placement form offers nodes with, so what
    // was offerable and what is dispatchable cannot drift apart — they
    // did, and this loop is where it showed.
    let allowed: Vec<String> =
        crate::infra::network::candidates_for(database, service.kind.placement_capability())
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
                let _ = crate::infra::repositories::replicas::set_last_error(
                    database,
                    &replica.id,
                    Some("that node has not agreed to keep data for this one"),
                )
                .await;
            }
            continue;
        }

        let holds_the_primary = slots.contains(&row.primary_slot);
        let holds_a_standby = slots.iter().any(|slot| *slot != row.primary_slot);
        let endpoint = match (&primary, holds_the_primary) {
            (_, true) => String::new(),
            (Some((host, port)), false) => format!("{host}:{port}"),
            (None, false) if holds_a_standby => continue,
            (None, false) => String::new(),
        };

        let payload = instruction(
            &project.name,
            service,
            &row,
            &domain,
            Holding {
                primary: endpoint,
                slots,
                running: running == crate::domain::entities::service::DesiredState::Running,
                replication_from: match holds_the_primary {
                    true => followers.clone(),
                    false => Vec::new(),
                },
                name: name.clone(),
                published: holds_the_primary && published,
            },
        )?;

        let queued = crate::infra::network::errand::queue_if_changed(
            database,
            &node_id,
            crate::infra::network::errand::Kind::Database,
            // One database, so a node holding standbys of two is told
            // about each without one overwriting the other's history.
            &format!("database:{}", service.id),
            &payload,
        )
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;

        if queued.is_some() {
            tracing::info!(
                service = %service.slug,
                node = %node_id,
                holds_the_primary,
                "asked a node to hold copies of a database"
            );
            sent += 1;
        }
        // After the database's own instruction, which is what makes the
        // copy there something a certificate can be for.
        if holds_the_primary {
            send_certificate(database, &project, service, &node_id).await?;
        }
    }
    Ok(sent)
}

/// The certificate the primary presents, to the node running it.
///
/// Decided here, by the node whose page shows it — ordered from an
/// authority, read from a file, or signed by this node — so the issuer and
/// the expiry on that page are the ones a client meets, and a client given
/// this node's authority verifies it. Its own instruction, because a
/// database's redeploys the copy and a renewal is a signal, not a restart.
async fn send_certificate(
    database: &SqliteDatabase,
    project: &crate::domain::entities::project::Project,
    service: &Service,
    node_id: &str,
) -> PlatformResult<()> {
    let names = crate::infra::deploy::certificate_names(database, project, service).await;
    let stored = crate::infra::deploy::database_certificate(database, &names)
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;
    let payload = serde_json::to_value(crate::infra::network::errand::Certificate {
        name: stored.domain.clone(),
        cert_pem: stored.cert_pem.clone(),
        key_pem: stored.key_pem.clone(),
        database: true,
    })
    .map_err(|error| PlatformError::Refused(error.to_string()))?;
    let queued = crate::infra::network::errand::queue_if_changed(
        database,
        node_id,
        crate::infra::network::errand::Kind::Certificate,
        &format!("database-certificate:{}", service.id),
        &payload,
    )
    .await
    .map_err(|error| PlatformError::Refused(error.to_string()))?;
    if queued.is_some() {
        tracing::info!(service = %service.slug, node = %node_id, "sent a database its certificate");
    }
    Ok(())
}

/// Who may follow a primary held on another node: the overlay address of
/// every node holding a standby, this one included.
///
/// The node holding the primary admits its own standbys by its bridge, as
/// any primary does, so it is left out.
async fn followers(
    database: &SqliteDatabase,
    row: &Database,
    placements: &[crate::domain::entities::replica::Replica],
    primary_node: Option<&str>,
) -> PlatformResult<Vec<String>> {
    if primary_node.is_none() {
        return Ok(Vec::new());
    }
    let nodes = crate::infra::network::all(database)
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;
    let mut from: Vec<String> = placements
        .iter()
        .filter(|replica| replica.slot != row.primary_slot && !replica.evicted())
        .filter(|replica| replica.node_id.as_deref() != primary_node)
        .filter_map(|replica| {
            nodes
                .iter()
                .find(|node| match &replica.node_id {
                    Some(id) => &node.id == id,
                    None => node.is_self,
                })
                .and_then(|node| node.overlay_ip.clone())
        })
        .map(|address| format!("{address}/32"))
        .collect();
    from.sort();
    from.dedup();
    Ok(from)
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
    domain: &Option<String>,
    holding: Holding,
) -> PlatformResult<serde_json::Value> {
    let Holding {
        primary,
        slots,
        running,
        replication_from,
        name,
        published,
    } = holding;
    serde_json::to_value(crate::infra::network::errand::Database {
        project: project.to_string(),
        service: service.name.clone(),
        image: service.image.clone(),
        registry: crate::domain::values::image_reference::ImageReference::parse(&service.image)
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
            .unwrap_or(crate::domain::policies::presets::SMALLEST),
        engine: row.engine.as_str().to_string(),
        version: row.version.clone(),
        database_name: row.database_name.clone(),
        admin_user: row.admin_user.clone(),
        admin_password: row.admin_password.clone(),
        replication_user: row.replication_user.clone(),
        replication_password: row.replication_password.clone(),
        primary,
        slots,
        primary_slot: row.primary_slot,
        replication_from,
        // The owner's, because the copy has to answer to the *database's*
        // qualified name rather than to one built from the domain of
        // whichever machine is holding it.
        qualified_domain: domain.clone(),
        name,
        published,
        running,
    })
    .map_err(|error| PlatformError::Refused(error.to_string()))
}

/// What one node is told it holds, apart from the database itself.
struct Holding {
    primary: String,
    slots: Vec<u32>,
    running: bool,
    replication_from: Vec<String>,
    name: Option<String>,
    published: bool,
}
/// Tell every node holding a copy of this database to let it go.
///
/// `slots: []`, which the holding node reads as "this is not yours to
/// keep any more": it stops the copy and deletes its own rows. It is the
/// one instruction a deletion can mean, and the only one that reaches the
/// node holding a primary with nothing for it to keep — a primary is
/// never moved.
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
    let project = crate::infra::repositories::projects::find(database, &service.project_id)
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
        &domain,
        Holding {
            primary: endpoint,
            slots: Vec::new(),
            running: false,
            replication_from: Vec::new(),
            name: None,
            published: false,
        },
    )?;

    let mut sent = 0;
    for node_id in crate::infra::repositories::replicas::of_service(database, &service.id)
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
        crate::infra::network::errand::queue(
            database,
            &node_id,
            crate::infra::network::errand::Kind::Database,
            &payload,
        )
        .await
        .map_err(|error| PlatformError::Refused(error.to_string()))?;
        tracing::info!(service = %service.slug, node = %node_id, "asked a node to let a copy go");
        sent += 1;
    }
    Ok(sent)
}
