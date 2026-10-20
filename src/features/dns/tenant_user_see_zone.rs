use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::dns_zone::DnsZone;
use crate::features::dns::describe_zone_command::DescribeZone;
use crate::features::dns::zone_response::ZoneResponse;
use crate::infra::repositories::zones;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    zone_name: &str,
) -> Result<ZoneResponse, String> {
    let zone = theirs(database, account, zone_name).await?;
    DescribeZone {
        zone: &zone,
        offer_servers: false,
    }
    .run(database)
    .await
}

/// The zone, if it is this organisation's — and the same answer as for a
/// zone that does not exist when it is somebody else's, so one
/// organisation cannot learn which domains another has.
pub async fn theirs(
    database: &SqliteDatabase,
    account: &Account,
    zone_name: &str,
) -> Result<DnsZone, String> {
    if !tenant_user::administers(account) {
        return Err("only an administrator of this organisation can do that".into());
    }
    let zone = zones::find_by_name(database, zone_name)
        .await
        .map_err(|error| {
            tracing::warn!(%error, zone = %zone_name, "could not read a zone");
            "the zones could not be read".to_string()
        })?
        .filter(|zone| {
            zone.is_ours() && zone.tenant_id.is_some() && zone.tenant_id == account.tenant_id
        })
        .ok_or_else(|| zones::NO_SUCH_ZONE.to_string())?;
    Ok(zone)
}
