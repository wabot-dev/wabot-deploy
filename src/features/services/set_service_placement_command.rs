use std::collections::BTreeSet;

use wabot::sqlite::SqliteDatabase;

use crate::config::Config;
use crate::deploy::quotas;
use crate::deploy::Deployer;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::repositories::replicas;
use wabot::prelude::Container;

/// Where copies added by the count control go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Placement {
    /// A machine somebody named. `None` is this one.
    Named(Option<String>),
    /// Wherever there is room, decided per copy — see
    /// `deploy::planner`.
    Anywhere,
}

/// What this command needs from the node it runs on.
///
/// Four things travel together through every step of a placement, and
/// naming them once keeps the signatures readable — and under clippy's
/// argument count, which is the same complaint in a lint.
#[derive(Clone, Copy)]
pub struct On<'a> {
    pub database: &'a SqliteDatabase,
    pub config: &'a Config,
    pub deployer: &'a Deployer,
    pub container: &'a Container,
}

/// Where a service's copies run, and how many there are.
///
/// **A replica is the unit.** A service is *n* placements, and a slot
/// number belongs to the service rather than the node it lands on.
pub struct SetServicePlacement<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    /// The form, read as `slot-{n}` → node id; empty means "here".
    pub fields: &'a std::collections::HashMap<String, String>,
    pub replicas: &'a str,
    /// `ANYWHERE`, empty for here, or a node id.
    pub on: &'a str,
    pub by: &'a str,
}

impl SetServicePlacement<'_> {
    pub async fn run(&self, on: On<'_>) -> Result<(), String> {
        let (database, deployer) = (on.database, on.deployer);
        let placements = replicas::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?;
        let mut touched: BTreeSet<String> = placements
            .iter()
            .filter_map(|replica| replica.node_id.clone())
            .collect();

        for replica in &placements {
            let Some(chosen) = self.fields.get(&format!("slot-{}", replica.slot)) else {
                continue;
            };
            let chosen = chosen.trim().to_string();
            let wanted = (!chosen.is_empty()).then_some(chosen.as_str());
            if wanted == replica.node_id.as_deref() {
                continue;
            }
            if let Some(node) = wanted {
                room_on(database, deployer, self.service, node).await?;
                touched.insert(node.to_string());
            }
            // Stopped before the row moves: the container id is derived
            // from the rows, so moving first loses the handle to what is
            // still running here.
            if replica.is_here() && wanted.is_some() {
                if let Err(error) = deployer
                    .stop_replica(self.project, self.service, replica)
                    .await
                {
                    tracing::warn!(
                        slot = replica.slot,
                        %error,
                        "stopping a copy that was sent to another node"
                    );
                }
            }
            replicas::move_to(database, &replica.id, wanted)
                .await
                .map_err(|error| error.to_string())?;
        }

        let wanted: u32 = self.replicas.parse().unwrap_or(1);
        if !(1..=16).contains(&wanted) {
            return Err("a service runs between 1 and 16 replicas".into());
        }

        let target = self.on.trim();
        let placement = match target {
            "anywhere" => Placement::Anywhere,
            "" => Placement::Named(None),
            node => {
                room_on(database, deployer, self.service, node).await?;
                touched.insert(node.to_string());
                Placement::Named(Some(node.to_string()))
            }
        };

        // The organisation's ceiling before the node's, for the same
        // reason as everywhere else: it is the more general refusal.
        if let Some(reason) = quotas::admits(
            database,
            self.project,
            self.service,
            self.service.memory_limit,
            self.service.cpu_millicores,
            self.service.disk_limit,
            u64::from(wanted),
        )
        .await
        .map_err(|error| error.to_string())?
        {
            return Err(reason);
        }

        let added = resize(
            database,
            deployer,
            self.project,
            self.service,
            wanted,
            &placement,
        )
        .await?;
        touched.extend(added);

        dispatch(on, self.project, self.service, self.by, touched).await
    }
}

async fn room_on(
    database: &SqliteDatabase,
    deployer: &Deployer,
    service: &Service,
    node: &str,
) -> Result<(), String> {
    let Some(row) = crate::network::find(database, node)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such node".into());
    };
    match deployer
        .room_for_on(
            Some(&row),
            service,
            service.memory_limit,
            service.cpu_millicores,
        )
        .await
    {
        Some(reason) => Err(reason),
        None => Ok(()),
    }
}

/// Add slots here, or take the highest-numbered ones away.
///
/// A replica on another node is refused rather than removed:
/// nothing can tell that node to stop yet, so dropping the row
/// would leave its container running with nothing naming it — the
/// worst of both, and invisible from either end.
async fn resize(
    database: &SqliteDatabase,
    deployer: &Deployer,
    project: &Project,
    service: &Service,
    wanted: u32,
    on: &Placement,
) -> Result<BTreeSet<String>, String> {
    let service_id = &service.id;
    let placements = crate::repositories::replicas::of_service(database, service_id)
        .await
        .map_err(|error| error.to_string())?;

    if placements.len() as u32 > wanted {
        // Which ones go, and the order is the point. By slot number
        // alone, lowering a service with a dead #2 and a healthy #3
        // removes the healthy one and keeps the corpse — so the
        // ones already thrown out where they ran go first, and only
        // then the highest-numbered.
        let mut going: Vec<_> = placements.iter().collect();
        going.sort_by_key(|replica| (replica.evicted(), replica.slot));
        going.reverse();
        going.truncate(placements.len() - wanted as usize);

        for replica in going {
            // A copy here is this node's to stop, and nothing else
            // would: reconciliation only ever *starts* things, so
            // dropping the row alone left a container running that
            // nothing named — the count said two and the machine
            // ran three.
            //
            // One elsewhere is stopped by the node running it, when
            // the `host` errand that follows this names the slots
            // that are left. That is why the set of nodes to tell
            // is taken *before* any of this: after it there is
            // nothing left pointing at the one that lost its last
            // copy. An evicted one is already stopped — that is
            // what evicting means.
            match replica.is_here() {
                // Stopped, its row dropped, and — for a standby —
                // the directory it held, which is what makes the
                // same slot come back as a fresh copy of the
                // primary rather than as this one's leftovers.
                true => deployer
                    .forget_replica(project, service, replica)
                    .await
                    .map_err(|error| error.to_string())?,
                false => crate::repositories::replicas::remove(database, &replica.id)
                    .await
                    .map_err(|error| error.to_string())?,
            }
        }
    }

    // By count, not by range: after a removal the service may hold
    // slots 1 and 3, which is already two copies. Filling
    // `1..=wanted` would put slot 2 back and undo the removal that
    // just happened — which is exactly what it did.
    let before: std::collections::BTreeSet<String> =
        crate::repositories::replicas::of_service(database, service_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter_map(|replica| replica.node_id)
            .collect();

    match on {
        Placement::Named(node) => {
            crate::repositories::replicas::ensure_count(
                database,
                service_id,
                wanted,
                node.as_deref(),
            )
            .await
            .map_err(|error| error.to_string())?;
        }
        Placement::Anywhere => {
            // Gathered once, and the copies added inside the loop are
            // counted back in by the closure — otherwise every new
            // copy lands on whichever machine was emptiest when the
            // form was submitted, which is the opposite of spreading
            // them.
            let candidates = deployer.candidates(service).await;
            let want = crate::deploy::planner::Want {
                memory: service.memory_limit,
                cpu: service.cpu_millicores,
                // A database's copies exist to be on different
                // machines; a web service's exist to be more of it.
                spread_required: service.kind.is_managed(),
            };
            let service_id_owned = service_id.clone();
            crate::repositories::replicas::ensure_count_where(
                database,
                service_id,
                wanted,
                |existing| {
                    let taken: Vec<&crate::entities::replica::Replica> = existing
                        .iter()
                        .filter(|replica| replica.service_id == service_id_owned)
                        .filter(|replica| !replica.evicted())
                        .collect();
                    let updated: Vec<crate::deploy::planner::Candidate> = candidates
                        .iter()
                        .cloned()
                        .map(|mut candidate| {
                            candidate.of_this_service = taken
                                .iter()
                                .filter(|replica| replica.node_id == candidate.node)
                                .count()
                                as u32;
                            candidate
                        })
                        .collect();
                    match crate::deploy::planner::choose(&updated, want) {
                        crate::deploy::planner::Choice::Chosen { node, why } => {
                            Ok((node, crate::entities::replica::Placed::Planner, Some(why)))
                        }
                        crate::deploy::planner::Choice::Nowhere(reason) => Err(reason),
                    }
                },
            )
            .await
            .map_err(|error| error.to_string())?;
        }
    }

    // Which machines are newly holding one, so the caller tells them.
    Ok(
        crate::repositories::replicas::of_service(database, service_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter_map(|replica| replica.node_id)
            .filter(|node| !before.contains(node))
            .collect(),
    )
}

/// Ask every node holding a replica of this service to run it, and
/// deploy the ones that are here.
///
/// Convergent on both sides: a `host` errand sent twice points one
/// service at one image there, and a local deploy is the deploy the
/// button already queues.
async fn dispatch(
    on: On<'_>,
    project: &Project,
    service: &Service,
    by: &str,
    touched: BTreeSet<String>,
) -> Result<(), String> {
    let (database, deployer, container) = (on.database, on.deployer, on.container);
    let placements = crate::repositories::replicas::of_service(database, &service.id)
        .await
        .map_err(|error| error.to_string())?;

    // Every node that holds a replica *or has just stopped holding
    // one*. The second half is why the set is passed in: a node
    // losing its last copy is exactly the one nothing points at any
    // more, and it is the one that most needs telling.
    let mut continue_managed = false;
    let touched_all = touched.clone();
    for node_id in touched {
        // Every slot that node holds, in the service's numbering:
        // a node running two copies is told about both in one
        // errand rather than being sent the same instruction twice
        // with no way to tell the copies apart.
        //
        // An **empty** list is a real instruction, not a reason to
        // skip: it says "you run none of this now", which is how a
        // node finds out that its last replica went home.
        let slots: Vec<u32> = placements
            .iter()
            .filter(|replica| replica.node_id.as_deref() == Some(node_id.as_str()))
            .filter(|replica| !replica.evicted())
            .map(|replica| replica.slot)
            .collect();

        // A managed database is **not** sent this way. A `host`
        // errand says "run this image here", and a receiving node
        // obeying it starts a Postgres with no volume, no engine
        // row and no TLS — initialising into a layer thrown away at
        // the next deployment, and looking like it worked. Its own
        // errand carries the rest, and `crate::commands::dispatch_standbys::dispatch` queues
        // it after the primary deploys, which is when the address it
        // has to carry exists.
        //
        // Found on a node: the placement page sent both, and the
        // database errand only won because it happened to arrive
        // second and is convergent.
        if service.kind.is_managed() {
            continue_managed = true;
            continue;
        }
        if let Err(error) =
            send_there(database, on.config, project, service, &node_id, by, slots).await
        {
            // Reported and not fatal: the other placements are
            // still worth making, and the page shows a replica
            // nobody has answered for.
            tracing::warn!(node = %node_id, %error, "could not queue a host errand");
        }
    }

    // A database's own errand, including to the nodes that just lost
    // their last copy — the `host` path above uses `touched` for
    // exactly that and this one had no equivalent, so a database
    // reduced from three copies to one left a Postgres running on the
    // other machine with nothing pointing at it.
    if continue_managed {
        deployer
            .dispatch_standbys_including(service, &touched_all)
            .await;
    }

    if placements.iter().any(|replica| replica.is_here()) {
        enqueue(container, &service.id, None).await;
    } else {
        // Nothing here to deploy, and the routes are wrong anyway:
        // a service whose last copy just left this node has a name
        // pointing at a container that is gone. The deployment is
        // what usually recomputes them, so with no deployment to
        // queue this has to say so itself — otherwise the name
        // proxies to an address nothing answers on, silently.
        deployer.sync_routes().await;
    }

    dispatch_edges(deployer, service, &placements).await?;
    Ok(())
}

/// One `host` errand, with a credential for this node's registry.
async fn send_there(
    database: &SqliteDatabase,
    config: &Config,
    project: &crate::entities::project::Project,
    service: &Service,
    node_id: &str,
    by: &str,
    slots: Vec<u32>,
) -> Result<(), String> {
    let Some(registry) = crate::values::image_reference::ImageReference::parse(&service.image)
        .ok()
        .and_then(|r| r.host())
    else {
        return Err(
            "that image does not name a registry, so there is nothing to pull it from".into(),
        );
    };

    // A token for **this node's** registry, and only when the image
    // is in it.
    //
    // The registry in the reference used to be sent a credential
    // whatever it was, which for `docker.io/library/postgres` means
    // handing a wabot push token to Docker Hub. It never happened
    // because nobody had placed a public image on another node —
    // and that is the ordinary case for a database, so it would
    // have.
    //
    // No credential is the honest payload for a registry that
    // serves anybody: the far node pulls anonymously, which is what
    // it would do with a credential Docker Hub does not know.
    let mine = crate::node::settings::domain(database, config).await;
    let secret = match mine.as_deref() == Some(registry.as_str()) {
        true => Some(
            crate::repositories::tokens::create(
                database,
                &project.id,
                &format!("errand to {node_id}"),
                by,
            )
            .await
            .map_err(|error| error.to_string())?
            .1,
        ),
        false => None,
    };

    let payload = serde_json::to_value(crate::network::errand::Host {
        project: project.name.clone(),
        service: service.name.clone(),
        image: service.image.clone(),
        registry,
        // Both or neither: a username with no secret is a
        // credential that cannot authenticate, and the collector
        // reads the pair.
        username: secret.as_ref().map(|_| "errand".to_string()),
        secret,
        env: service.env.clone(),
        // The port a name is served on, so the node running the
        // copy has something to bind on its overlay address. Left
        // empty when no port has a hostname: nothing would proxy to
        // that service anyway, so a port there would be one nobody
        // asked for.
        port: crate::repositories::ports::of_service(database, &service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|port| port.hostname.is_some())
            .map(|port| port.container_port),
        slots,
        // Placing a copy is asking for it to run. Whether the service
        // stays running afterwards travels on its own, derived from
        // the row — see `Deployer::tell_holders`.
        running: true,
        // The ceilings it runs under. Sent from the first
        // instruction rather than left for the derived pass to
        // correct, so the copy is never started once without them.
        memory_limit: service.memory_limit,
        cpu_millicores: service.cpu_millicores,
        disk_limit: service.disk_limit,
    })
    .map_err(|error| error.to_string())?;

    crate::network::errand::queue(
        database,
        node_id,
        crate::network::errand::Kind::Host,
        &payload,
    )
    .await
    .map(|_| ())
    .map_err(|error| error.to_string())
}

/// Tell every node serving a name where its replicas are now.
///
/// Sent whenever the placement changes, because that is when the
/// answer changes: a replica moved, added or gone is a different
/// upstream list, and an edge holding the old one would send
/// requests to a container that is not there.
/// Everyone else who answers for this name, as the node being told
/// will need them.
///
/// Skipping the recipient itself, and skipping any node this one
/// cannot describe: an entry with no overlay address, no key or no
/// authority is one the far node could not dial anyway, and a peer it
/// cannot verify is one `call::to_node` refuses by design.
///
/// **This node's own authority comes from the local CA**, not from its
/// row: `ca_pem` is what *other* nodes sent about themselves, so the
/// self row has none.
/// Tell every node serving a name where its replicas are now, and
/// bring this node's own routes up to date.
///
/// Thin over `Deployer::tell_edges`, which is also called by every
/// deployment — the console's reason for calling it is latency: an
/// operator who just ticked a box should not wait for the next
/// deployment to have it happen. Both build the same payload, so
/// `queue_if_changed` keeps the second one quiet.
pub(crate) async fn dispatch_edges(
    deployer: &Deployer,
    service: &Service,
    _placements: &[crate::entities::replica::Replica],
) -> Result<(), String> {
    deployer.tell_edges(service).await;
    // And this node's own, whether or not it was chosen: the choice
    // that just changed may have been to stop answering, and a route
    // left behind is a name still being served by a node nobody asked.
    deployer.sync_routes().await;
    Ok(())
}

async fn enqueue(container: &Container, service_id: &str, release_id: Option<&str>) {
    let command = crate::deploy::jobs::DeployService {
        service_id: service_id.to_string(),
        release_id: release_id.map(str::to_string),
    };
    if let Err(error) = wabot::async_jobs::run_command(container, &command).await {
        // Nothing else can report this: there is no job to carry the
        // reason, because making one is what failed.
        tracing::error!(%service_id, %error, "could not queue a deployment");
    }
}
