//! What one organisation has undertaken, and whether it may undertake
//! more.
//!
//! ## Two sums, two names
//!
//! `deploy::Committed` is what **this machine** has promised, summed
//! over the copies it runs. This is what **one tenant** has promised,
//! summed over its copies wherever they run. They are different
//! questions with the same units, and giving them one name is how
//! somebody passes one where the other belongs.
//!
//! And they compose rather than compete: a copy is charged to its
//! tenant's budget *and* against the machine holding it, because both
//! are real. Neither is a share of the other.
//!
//! ## Why the tenant's sum needs no network
//!
//! Every row of a tenant lives on this node — that is what a tenant *is*
//! here, and the reason the plan chose one node over a replicated pool.
//! So the sum is a local read, always answerable, and never stale. The
//! node's half is the one that has to travel and can be missing; this
//! one cannot.
//!
//! ## What a test here cannot reach
//!
//! The two forms that ask this — the resources form and the placement
//! count — are reached in a tenant's scope, and the scope is stamped on
//! the request by `edge::dispatch` from the route the `Host` resolved
//! to. The console's own test harness drives the UI router directly and
//! never goes through the edge, so no request it makes carries a
//! `Tenancy` and no test at that level can be in a tenant's scope at
//! all.
//!
//! So the arithmetic and the ordering are pinned here, where they are a
//! function of rows; that the forms ask it is a node run with somebody
//! clicking. Writing a console test for it would have meant a fixture
//! that fakes the scope, which is a test agreeing with itself.
//!
//! ## A quota demands ceilings
//!
//! A node cannot refuse a limitless container that is already running,
//! so `Committed::unbounded` is reported beside its sum and folded into
//! nothing. A tenant can and must: an organisation with a quota and a
//! service with no `memory_limit` has a quota that is decoration, and
//! the refusal says which service rather than making somebody guess.

use wabot::sqlite::SqliteDatabase;

use super::PlatformResult;
use crate::deploy::{Room, Rooms};

/// What one tenant has undertaken, over every copy anywhere.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Charged {
    pub memory: u64,
    pub cpu: u32,
    pub disk: u64,
    /// Services of this tenant with no memory ceiling at all.
    ///
    /// Named rather than summed, and unlike the node's version this one
    /// is **acted on**: see the module note.
    pub unbounded: usize,
}

/// Add up what a tenant is holding.
///
/// Per **replica**, not per service: three copies of a 256 MB service
/// cost 768 MB, which is what they actually cost. Evicted copies do not
/// count — the node holding one threw it off, and nothing is running.
pub async fn charged(database: &SqliteDatabase, tenant_id: &str) -> PlatformResult<Charged> {
    let projects = super::access::of_tenant(database, tenant_id).await?;
    let mut charged = Charged::default();
    for project in projects {
        for service in crate::repositories::services::all(database, Some(&project.id)).await? {
            let copies = super::replicas::of_service(database, &service.id)
                .await?
                .into_iter()
                .filter(|replica| !replica.evicted())
                .count() as u64;
            match service.memory_limit {
                Some(bytes) => charged.memory += bytes.saturating_mul(copies),
                None => charged.unbounded += 1,
            }
            if let Some(millicores) = service.cpu_millicores {
                charged.cpu = charged
                    .cpu
                    .saturating_add(millicores.saturating_mul(copies as u32));
            }
            if let Some(bytes) = service.disk_limit {
                charged.disk = charged.disk.saturating_add(bytes.saturating_mul(copies));
            }
        }
    }
    Ok(charged)
}

/// The tenant's two ceilings, for [`crate::deploy::fits`].
///
/// `None` when this project belongs to no tenant, which is every project
/// the node owns itself — there is no second budget to check, and the
/// node's own arithmetic is the only one.
pub async fn rooms_for(
    database: &SqliteDatabase,
    project: &crate::entities::project::Project,
    service: &crate::entities::service::Service,
) -> PlatformResult<Option<(Rooms, String)>> {
    let Some(tenant_id) = &project.tenant_id else {
        return Ok(None);
    };
    let Some(tenant) = super::tenants::find(database, tenant_id).await? else {
        return Ok(None);
    };
    let charged = charged(database, tenant_id).await?;
    Ok(Some((
        Rooms {
            memory: Room {
                // **`known` is "there is a quota"**, not "we could
                // measure". A tenant without one is not refused
                // anything, which is the same rule the unreadable meter
                // follows and what lets this migration land on a running
                // node without narrowing anybody.
                allocatable: tenant.quota_memory.unwrap_or(0),
                known: tenant.quota_memory.is_some(),
                committed: charged.memory,
                already: service.memory_limit.unwrap_or(0),
            },
            cpu: Room {
                allocatable: u64::from(tenant.quota_cpu.unwrap_or(0)),
                known: tenant.quota_cpu.is_some(),
                committed: u64::from(charged.cpu),
                already: u64::from(service.cpu_millicores.unwrap_or(0)),
            },
        },
        tenant.name,
    )))
}

/// Whether this tenant may take this much more, and why not.
///
/// **Asked before the node's**, and the order is the answer rather than
/// a preference. Three reasons and the third decides: this sum is a
/// local read and always answerable where the node's may be missing or
/// stale; it is the more general refusal, since a tenant out of budget
/// is out of budget on every machine; and a refusal should lead to the
/// smallest correction — "ask for more quota" before "put it somewhere
/// else", because the second is advice that cannot work.
pub async fn admits(
    database: &SqliteDatabase,
    project: &crate::entities::project::Project,
    service: &crate::entities::service::Service,
    memory: Option<u64>,
    cpu: Option<u32>,
    disk: Option<u64>,
    copies: u64,
) -> PlatformResult<Option<String>> {
    let Some((rooms, name)) = rooms_for(database, project, service).await? else {
        return Ok(None);
    };

    // A quota only means something if the things under it have ceilings.
    // Named service by service, because "some service has no limit" is
    // not something anybody can act on.
    if rooms.memory.known || rooms.cpu.known {
        if let Some(offender) = limitless(database, project, service).await? {
            return Ok(Some(format!(
                "{name} has a quota, so its services need a ceiling — {offender} has none"
            )));
        }
    }

    if let Some(refusal) = rooms.admits(memory, cpu, copies, &name) {
        return Ok(Some(refusal));
    }

    // **Disk is checked here and not through `Rooms`**, and the reason is
    // that `Rooms` is the pair a *node* answers with: it is built from
    // `network::Capacity`, which deliberately carries no disk because
    // nothing summed a disk commitment until the column existed. An
    // organisation's budget is a local sum and can answer now; a remote
    // machine still cannot, and widening the shared struct with a field
    // one of its two builders could only fill with zero would make every
    // node look like one with no disk spoken for.
    let Some(wanted) = disk else {
        return Ok(None);
    };
    let Some(tenant_id) = &project.tenant_id else {
        return Ok(None);
    };
    let Some(tenant) = super::tenants::find(database, tenant_id).await? else {
        return Ok(None);
    };
    let charged = charged(database, tenant_id).await?;
    Ok(crate::deploy::fits(
        Room {
            allocatable: tenant.quota_disk.unwrap_or(0),
            known: tenant.quota_disk.is_some(),
            committed: charged.disk,
            already: service.disk_limit.unwrap_or(0),
        },
        wanted,
        copies,
        &name,
        crate::node::memory::human,
    ))
}

/// The first service of this tenant with no memory ceiling, if any.
///
/// The service being changed is exempt: it is about to get one, and
/// refusing the form that would give it one is the kind of rule that
/// cannot be satisfied from the page it appears on.
async fn limitless(
    database: &SqliteDatabase,
    project: &crate::entities::project::Project,
    changing: &crate::entities::service::Service,
) -> PlatformResult<Option<String>> {
    let Some(tenant_id) = &project.tenant_id else {
        return Ok(None);
    };
    for project in super::access::of_tenant(database, tenant_id).await? {
        for service in crate::repositories::services::all(database, Some(&project.id)).await? {
            if service.id != changing.id && service.memory_limit.is_none() {
                return Ok(Some(service.slug));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    async fn tenant_with_a_service() -> (
        SqliteDatabase,
        crate::entities::project::Project,
        crate::entities::service::Service,
    ) {
        let database = crate::db::open_in_memory().await.expect("open");
        let tenant = super::super::tenants::create(&database, "Acme")
            .await
            .expect("tenant");
        let project =
            crate::repositories::projects::create_for(&database, "theirs", Some(&tenant.id))
                .await
                .expect("project");
        let service = crate::commands::create_service::CreateService {
            project_id: &project.id,
            name: "web",
            image: "hub.example/theirs/web@sha256:a",
            env: &[],
        }
        .run(&database)
        .await
        .expect("service");
        super::super::tenants::set_quota(&database, &tenant.id, Some(512 * MB), None, None)
            .await
            .expect("quota");
        (database, project, service)
    }

    /// What a tenant holds is counted per copy, wherever the copies run.
    ///
    /// A service with three replicas costs three times its ceiling, and
    /// two of those being on other machines changes nothing: the budget
    /// is the organisation's, and where the work landed is the node's
    /// question rather than theirs.
    #[tokio::test]
    async fn a_tenants_budget_is_charged_per_copy_wherever_it_runs() {
        let (database, project, service) = tenant_with_a_service().await;
        crate::repositories::services::set_memory_limit(&database, &service.id, Some(128 * MB))
            .await
            .expect("ceiling");
        // Slot 1 exists already: creating a service places one copy
        // here. Two more, on two other machines.
        super::super::replicas::place(&database, &service.id, Some("nd-elsewhere1"), 2)
            .await
            .expect("there");
        super::super::replicas::place(&database, &service.id, Some("nd-elsewhere2"), 3)
            .await
            .expect("further");

        let tenant_id = project.tenant_id.clone().expect("a tenant");
        assert_eq!(
            charged(&database, &tenant_id)
                .await
                .expect("charged")
                .memory,
            384 * MB,
            "three copies of 128 MB"
        );
    }

    /// Without a quota nothing is refused.
    ///
    /// The rule that lets this land on a running node: every tenant that
    /// exists the moment the column does has `NULL`, and a default would
    /// be a migration deciding somebody's organisation is now refused at
    /// a number nobody chose.
    #[tokio::test]
    async fn a_tenant_without_a_quota_is_refused_nothing() {
        let (database, project, service) = tenant_with_a_service().await;
        let tenant_id = project.tenant_id.clone().expect("a tenant");
        super::super::tenants::set_quota(&database, &tenant_id, None, None, None)
            .await
            .expect("lifted");

        assert_eq!(
            admits(&database, &project, &service, Some(u64::MAX), None, None, 1)
                .await
                .expect("asked"),
            None,
            "a rule made of a missing number is not a rule"
        );
    }

    /// Over the ceiling is refused, with the numbers and the name.
    #[tokio::test]
    async fn a_tenant_over_its_ceiling_is_refused_and_told_by_how_much() {
        let (database, project, service) = tenant_with_a_service().await;
        crate::repositories::services::set_memory_limit(&database, &service.id, Some(128 * MB))
            .await
            .expect("ceiling");
        let service = crate::repositories::services::find(&database, &service.id)
            .await
            .expect("query")
            .expect("row");

        // Its own 128 MB does not count against it, so 512 fits exactly.
        assert_eq!(
            admits(&database, &project, &service, Some(512 * MB), None, None, 1)
                .await
                .expect("asked"),
            None
        );

        let refusal = admits(&database, &project, &service, Some(768 * MB), None, None, 1)
            .await
            .expect("asked")
            .expect("refused");
        assert!(refusal.starts_with("Acme has"), "{refusal}");
        assert!(refusal.contains("512 MB to promise"), "{refusal}");
    }

    /// A quota demands ceilings, and the refusal names the service.
    ///
    /// Unlike a node, which cannot refuse a limitless container that is
    /// already running, an organisation with a budget and a service with
    /// no ceiling has a budget that means nothing — the sum would be a
    /// floor and the page would show it as a total.
    #[tokio::test]
    async fn a_quota_demands_that_its_services_have_ceilings() {
        let (database, project, service) = tenant_with_a_service().await;
        let loose = crate::commands::create_service::CreateService {
            project_id: &project.id,
            name: "worker",
            image: "hub.example/theirs/worker@sha256:b",
            env: &[],
        }
        .run(&database)
        .await
        .expect("service");

        let refusal = admits(&database, &project, &service, Some(64 * MB), None, None, 1)
            .await
            .expect("asked")
            .expect("refused");
        assert!(refusal.contains("need a ceiling"), "{refusal}");
        assert!(refusal.contains(&loose.slug), "{refusal}");

        // And giving it one clears the way — a rule that could not be
        // satisfied would be worse than none.
        crate::repositories::services::set_memory_limit(&database, &loose.id, Some(64 * MB))
            .await
            .expect("ceiling");
        assert_eq!(
            admits(&database, &project, &service, Some(64 * MB), None, None, 1)
                .await
                .expect("asked"),
            None
        );
    }

    /// Disk is charged and refused like the other two, and enforced like
    /// neither.
    ///
    /// **The number is real; what it does is different.** Memory and CPU
    /// reach a cgroup; there is no `disk.max` to write, so this stops the
    /// next placement and does not stop a container already running. The
    /// form says so — a field that looked like the other two and quietly
    /// did less would be the `archiving` label again.
    #[tokio::test]
    async fn disk_is_charged_against_the_budget_even_though_nothing_enforces_it() {
        let (database, project, service) = tenant_with_a_service().await;
        let tenant_id = project.tenant_id.clone().expect("a tenant");
        super::super::tenants::set_quota(&database, &tenant_id, None, None, Some(512 * MB))
            .await
            .expect("a disk budget and nothing else");
        // A quota demands ceilings, so give the one service its own.
        crate::repositories::services::set_memory_limit(&database, &service.id, Some(64 * MB))
            .await
            .expect("ceiling");
        crate::repositories::services::set_disk_limit(&database, &service.id, Some(128 * MB))
            .await
            .expect("disk");
        super::super::replicas::place(&database, &service.id, Some("nd-elsewhere1"), 2)
            .await
            .expect("a second copy");

        // Two copies at 128 MB.
        assert_eq!(
            charged(&database, &tenant_id).await.expect("charged").disk,
            256 * MB,
            "disk is charged per copy like the other two"
        );

        let service = crate::repositories::services::find(&database, &service.id)
            .await
            .expect("query")
            .expect("row");

        // Raising it to 256 each is 512 across two copies, which fits
        // exactly; 384 each does not.
        assert_eq!(
            admits(&database, &project, &service, None, None, Some(256 * MB), 2)
                .await
                .expect("asked"),
            None
        );
        let refusal = admits(&database, &project, &service, None, None, Some(384 * MB), 2)
            .await
            .expect("asked")
            .expect("refused");
        assert!(refusal.starts_with("Acme has"), "{refusal}");
        assert!(refusal.contains("512 MB to promise"), "{refusal}");
    }

    /// A project the node owns itself has no second budget.
    #[tokio::test]
    async fn the_nodes_own_projects_answer_to_nobodys_quota() {
        let database = crate::db::open_in_memory().await.expect("open");
        let project = crate::repositories::projects::create(&database, "ours")
            .await
            .expect("project");
        let service = crate::commands::create_service::CreateService {
            project_id: &project.id,
            name: "web",
            image: "hub.example/ours/web@sha256:a",
            env: &[],
        }
        .run(&database)
        .await
        .expect("service");

        assert!(rooms_for(&database, &project, &service)
            .await
            .expect("asked")
            .is_none());
        assert_eq!(
            admits(&database, &project, &service, Some(u64::MAX), None, None, 1)
                .await
                .expect("asked"),
            None
        );
    }
}
