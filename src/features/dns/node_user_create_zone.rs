use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::zone_name::ZoneName;
use crate::features::dns::create_zone_command::CreateZone;
use crate::features::dns::create_zone_request::CreateZoneRequest;
use crate::features::dns::zone_created_response::ZoneCreatedResponse;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &CreateZoneRequest,
) -> Result<ZoneCreatedResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let name = ZoneName::parse(&asked.name)?;
    CreateZone {
        name: &name,
        tenant_id: None,
        with_every_name_server: false,
    }
    .run(database, container)
    .await
}
