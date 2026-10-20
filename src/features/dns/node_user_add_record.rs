use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::dns::add_record_command::AddRecord;
use crate::features::dns::add_record_request::AddRecordRequest;
use crate::infra::repositories::zones;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &AddRecordRequest,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let zone = zones::find_by_name(database, &asked.zone)
        .await
        .map_err(|error| {
            tracing::warn!(%error, "could not read a zone");
            "the zones could not be read".to_string()
        })?
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;
    AddRecord { zone: &zone, asked }
        .run(database, container)
        .await
}
