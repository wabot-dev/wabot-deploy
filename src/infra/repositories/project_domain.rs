//! What domain a project's names hang off.
//!
//! **Here rather than in tenancy, and that is the point.** Its three
//! callers ask about a *project*, which is a thing this node has with or
//! without organisations — so none of them has to name one, and deleting
//! tenancy simplifies this instead of touching them.

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;

/// The domain this project's names hang off.
///
/// Its organisation's when it has one, and this node's otherwise —
/// which is every project the node owns itself, and every project that
/// existed before organisations did.
///
/// **Here rather than in `tenants`, and that is the point.** Its three
/// callers ask about a *project*, which is a thing this node has with
/// or without organisations — so none of them has to name one, and
/// deleting tenancy simplifies this function instead of touching them.
///
/// **One function because there are three callers and they must not
/// disagree**: the name a page proposes, the name a certificate is
/// ordered for, and the name written into a container's `/etc/hosts`.
/// Two of those drifting once already produced a client dialling a name
/// no certificate held — see `certificate_names`.
///
/// It does **not** override `database.owner_domain`. That column answers
/// a different question — whose name is this copy known by, when the
/// copy is held for somebody else — and it wins wherever it is set: a
/// database placed on another node keeps the name its owner gave it, and
/// that owner may be an organisation on a third machine entirely.
pub async fn domain_for(
    database: &SqliteDatabase,
    project: &Project,
    node_domain: Option<&str>,
) -> Option<String> {
    let theirs = match &project.tenant_id {
        Some(tenant_id) => crate::infra::repositories::tenants::find(database, tenant_id)
            .await
            .ok()
            .flatten()
            .and_then(|tenant| tenant.domain),
        None => None,
    };
    theirs.or_else(|| node_domain.map(str::to_string))
}

/// The domain a page proposes a project's **public** names under.
///
/// Not [`domain_for`], which also names what only the project's own
/// containers resolve: an organisation's project whose organisation has no
/// domain is proposed nothing, rather than a name under the node's — which
/// would put its pages on the same site as the node's console.
pub async fn public_domain_for(
    database: &SqliteDatabase,
    project: &Project,
    node_domain: Option<&str>,
) -> Option<String> {
    match &project.tenant_id {
        Some(_) => domain_for(database, project, None).await,
        None => node_domain.map(str::to_string),
    }
}

/// Refuse a public name an organisation's project may not answer on.
///
/// An invariant of the project rather than a question of who is asking:
/// the node's operator naming a tenant's service under the node's domain
/// puts the same page on the same site as the console.
pub async fn refuse_a_foreign_name(
    database: &SqliteDatabase,
    project_id: &str,
    hostname: &str,
) -> Result<(), String> {
    let project = crate::infra::repositories::projects::find(database, project_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("no such project")?;
    let Some(tenant_id) = &project.tenant_id else {
        return Ok(());
    };
    let tenants = crate::infra::repositories::tenants::all(database)
        .await
        .map_err(|error| error.to_string())?;
    let own = tenants
        .iter()
        .find(|tenant| &tenant.id == tenant_id)
        .and_then(|tenant| tenant.domain.clone());
    let others: Vec<String> = tenants
        .iter()
        .filter(|tenant| &tenant.id != tenant_id)
        .filter_map(|tenant| tenant.domain.clone())
        .collect();
    let node_domain = crate::infra::node::settings::domain(database).await;
    match crate::domain::policies::tenant_names::refusal(
        hostname,
        node_domain.as_deref(),
        own.as_deref(),
        &others,
    ) {
        Some(reason) => Err(reason.to_string()),
        None => Ok(()),
    }
}
