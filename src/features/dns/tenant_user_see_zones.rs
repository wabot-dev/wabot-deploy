use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::features::dns::describe_zones_command::DescribeZones;
use crate::features::dns::zones_response::ZonesResponse;
use crate::infra::repositories::access;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    project: &str,
) -> Result<ZonesResponse, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to see".into());
    }
    let (project, allowed) = access::find_project(database, account, project)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not read a project");
            "the zones could not be read".to_string()
        })?
        .ok_or_else(|| "no such project".to_string())?;
    DescribeZones {
        project: &project,
        editable: allowed.may_deploy() && project.is_ours(),
    }
    .run(database)
    .await
}
