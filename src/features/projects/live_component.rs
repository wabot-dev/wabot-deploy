//! What a project's page paints first, and what its stream sends after.
//!
//! **One builder, read by both controllers**, which is the whole reason
//! it is not in either: the page renders it on the first paint and the
//! stream sends the same shape every two seconds, and two copies of
//! that is two ways for a badge to disagree with itself.
//!
//! It reads the database, so it is not a view — the cells it fills are
//! `views::components::project_cells`, which are pure.

use crate::features::services::project_cells_component::{
    edge_cell, replica_cell, EdgeCell, NameCell, ReplicaCell,
};
use crate::features::services::service_state_component::StateCell;

/// Everything on a project's pages that changes without a click.
///
/// Three maps rather than one, because they are keyed by three
/// different things and a page shows some or all of them: the overview
/// has services, the service page has services *and* its replicas and
/// its names.
///
/// It grew from the services map alone, and the gap was the point: a
/// replica placed on another node and a certificate being issued are
/// exactly the two things somebody presses a button and then waits for,
/// and both left the page frozen with a badge that had been true when
/// it rendered.
#[derive(serde::Serialize)]
pub struct Live {
    pub services: std::collections::BTreeMap<String, StateCell>,
    pub replicas: std::collections::BTreeMap<String, ReplicaCell>,
    pub names: std::collections::BTreeMap<String, NameCell>,
    pub edges: std::collections::BTreeMap<String, EdgeCell>,
}

impl Live {
    /// Every word in here, in the reader's language.
    ///
    /// The words are chosen where the state is decided — one place, so a
    /// first paint and an update cannot disagree — and that place has no
    /// business knowing who is reading. So they are English there and
    /// translated here, on the way out.
    pub fn translate(&mut self) {
        for cell in self.services.values_mut() {
            cell.word = crate::features::services::texts::word(&cell.word).to_string();
            // The one field composed rather than looked up. Said here
            // and rendered by `say` on the page, so a count sent two
            // seconds after a page load cannot be in the other
            // language.
            cell.address = cell.whereabouts.say();
        }
        for cell in self.replicas.values_mut() {
            cell.word = crate::features::services::texts::word(&cell.word).to_string();
            cell.detail = crate::features::services::texts::word(&cell.detail).to_string();
        }
        for cell in self.edges.values_mut() {
            cell.word = crate::features::services::texts::word(&cell.word).to_string();
        }
    }
}

pub async fn replica_cells(
    state: &crate::shell::state::ConsoleState,
    project: &crate::domain::entities::project::Project,
    // What each container here is using, from two readings of the stream.
    // `None` on the first tick, which is why the cell is empty rather than
    // a dash: the page keeps what it has instead of flashing.
    busy: Option<&crate::infra::node::cpu::Busy>,
) -> std::collections::BTreeMap<String, ReplicaCell> {
    let mut cells = std::collections::BTreeMap::new();
    let Ok(services) =
        crate::infra::repositories::services::all(&state.database, Some(&project.id)).await
    else {
        return cells;
    };

    // Which nodes have an instruction waiting to be collected, once for the
    // project rather than once per replica.
    let queued = crate::infra::network::errand::all(&state.database)
        .await
        .unwrap_or_default();

    for service in services {
        let Ok(replicas) =
            crate::infra::repositories::replicas::of_service(&state.database, &service.id).await
        else {
            continue;
        };
        let stopped =
            service.desired_state == crate::domain::entities::service::DesiredState::Stopped;
        // Which of its copies the edge has taken out of the rotation.
        // Once per service rather than once per replica: it is one read
        // of the health map and one derivation of the addresses.
        let silent = state.deployer.not_answering(&service).await;
        for replica in replicas {
            let waiting_for_it = replica.node_id.as_ref().is_some_and(|node| {
                queued.iter().any(|record| {
                    record.done_at.is_none()
                        && &record.node_id == node
                        && record.payload.get("service").and_then(|name| name.as_str())
                            == Some(service.name.as_str())
                })
            });
            let mut cell = replica_cell(
                &replica,
                stopped,
                waiting_for_it,
                silent.contains(&replica.id),
            );
            // Only the copies here. One elsewhere is measured by the node
            // running it and arrives on its report, which the server has
            // already rendered — writing over it from a stream that has no
            // reading for it would blank a figure that is true.
            if replica.is_here() && !replica.evicted() {
                let id = replica.container_id(&project.slug, &service.slug);
                if let Some(share) = busy.and_then(|busy| busy.containers.get(&id)) {
                    // A thousand is one core. Absolute, so this column can
                    // be read against the same column on another node.
                    cell.cpu = format!("{share}m");
                }
            }
            cells.insert(replica.id.clone(), cell);
        }
    }
    cells
}

/// How far each edge instruction has got.
///
/// Keyed `hostname|node`, which is what the row is: the same name can be
/// asked of several nodes and each answers on its own.
///
/// Read from the errand, because the errand is the only thing that
/// crosses. A tick used to be the end of what the page said — an errand
/// went out, a name was claimed and a certificate ordered, and none of
/// it came back — so the honest reading of a ticked box was "somebody
/// asked for this once".
pub async fn edge_cells(
    state: &crate::shell::state::ConsoleState,
    project: &crate::domain::entities::project::Project,
) -> std::collections::BTreeMap<String, EdgeCell> {
    let mut cells = std::collections::BTreeMap::new();
    let Ok(services) =
        crate::infra::repositories::services::all(&state.database, Some(&project.id)).await
    else {
        return cells;
    };
    let orders = crate::infra::network::errand::all(&state.database)
        .await
        .unwrap_or_default();

    for service in services {
        let Ok(chosen) =
            crate::infra::repositories::edges::of_service(&state.database, &service.id).await
        else {
            continue;
        };
        for (hostname, node_id) in chosen {
            // The newest instruction to that node about that name.
            // Older ones are history and saying "done" from one of them
            // while a newer sits pending would be the page reporting the
            // wrong round trip.
            let latest = orders
                .iter()
                .filter(|order| order.node_id == node_id)
                .filter(|order| order.kind == crate::infra::network::errand::Kind::Edge)
                .filter(|order| {
                    order
                        .payload
                        .get("hostname")
                        .and_then(|value| value.as_str())
                        == Some(hostname.as_str())
                })
                .max_by_key(|order| order.created_at);

            cells.insert(format!("{hostname}|{node_id}"), edge_cell(latest));
        }
    }
    cells
}

/// Which of this project's hostnames are still waiting for a
/// certificate.
///
/// The badge under an address used to be true only at the moment the
/// page rendered, and an ACME order takes minutes — so the one thing
/// somebody stares at after adding a hostname was the one thing that
/// never changed on its own.
pub async fn name_cells(
    state: &crate::shell::state::ConsoleState,
    project: &crate::domain::entities::project::Project,
) -> std::collections::BTreeMap<String, NameCell> {
    let mut cells = std::collections::BTreeMap::new();
    let Ok(services) =
        crate::infra::repositories::services::all(&state.database, Some(&project.id)).await
    else {
        return cells;
    };

    for service in services {
        let Ok(ports) =
            crate::infra::repositories::ports::of_service(&state.database, &service.id).await
        else {
            continue;
        };
        for hostname in ports.into_iter().filter_map(|port| port.hostname) {
            let secured = crate::infra::edge::certs::settled(&state.database, &hostname).await;
            cells.insert(hostname, NameCell { waiting: !secured });
        }
    }
    cells
}
