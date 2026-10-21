use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::dns_zone::DnsZone;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::Access;
use crate::features::dns::describe_zone_command::DescribeZone;
use crate::features::dns::zone_response::ZoneResponse;
use crate::infra::repositories::{access, zones};

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &str,
    zone_name: &str,
) -> Result<ZoneResponse, String> {
    let (_, allowed, zone) = theirs(database, account, project, zone_name).await?;
    DescribeZone {
        zone: &zone,
        offer_servers: false,
        editable: allowed.may_deploy(),
    }
    .run(database)
    .await
}

/// The zone, if it is in a project this person can see — and the same
/// answer as for a zone that does not exist when it is in another one, so
/// nobody learns which domains a project they are not in keeps.
pub async fn theirs(
    database: &SqliteDatabase,
    account: &Account,
    project: &str,
    zone_name: &str,
) -> Result<(Project, Access, DnsZone), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to see".into());
    }
    let unreadable = |error: String| {
        tracing::warn!(%error, zone = %zone_name, "could not read a zone");
        "the zones could not be read".to_string()
    };
    let (project, allowed) = access::find_project(database, account, project)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .ok_or_else(|| "no such project".to_string())?;
    let zone = zones::find_by_name(database, zone_name)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .filter(|zone| zone.project_id.as_deref() == Some(project.id.as_str()))
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;
    Ok((project, allowed, zone))
}
