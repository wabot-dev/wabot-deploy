use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::dns::delete_zone_command::DeleteZone;
use crate::infra::repositories::zones;

/// Put down a zone no project here holds. A project's zone is deleted from
/// its project, by its people, so it reads here as one that does not exist.
pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    zone_name: &str,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let zone = zones::find_by_name(database, zone_name)
        .await
        .map_err(|error| {
            tracing::warn!(%error, zone = %zone_name, "could not read a zone");
            "the zones could not be read".to_string()
        })?
        .filter(|zone| zone.project_id.is_none())
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;
    DeleteZone { zone: &zone }.run(database, container).await
}
