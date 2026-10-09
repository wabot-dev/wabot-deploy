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
