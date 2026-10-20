use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
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
    if !tenant_user::administers(account) {
        return Err("only an administrator of this organisation can do that".into());
    }
    let name = ZoneName::parse(&asked.name)?;
    CreateZone {
        name: &name,
        tenant_id: account.tenant_id.as_deref(),
        with_every_name_server: true,
    }
    .run(database, container)
    .await
}

#[cfg(test)]
mod tests {
    use crate::features::dns::add_record_request::AddRecordRequest;
    use crate::features::dns::create_zone_request::CreateZoneRequest;
    use crate::infra::config::Config;
    use crate::infra::repositories::zones;
    use wabot::prelude::Container;

    async fn acme_with_a_domain() -> crate::features::fixtures::Organisation {
        let organisation = crate::features::fixtures::organisation().await;
        crate::infra::network::ensure_self(&organisation.database, &Config::default())
            .await
            .expect("this node");
        super::run(
            &organisation.database,
            &Container::new(),
            &organisation.admin,
            &CreateZoneRequest {
                name: "acme.example".into(),
            },
        )
        .await
        .expect("acme's domain");
        organisation
    }

    #[tokio::test]
    async fn an_organisation_adds_its_own_domain() {
        let organisation = acme_with_a_domain().await;
        let zone = zones::find_by_name(&organisation.database, "acme.example")
            .await
            .expect("read")
            .expect("there");
        assert_eq!(zone.tenant_id, organisation.admin.tenant_id);
        assert!(zone.is_ours());
        assert_eq!(
            zones::servers(&organisation.database, &zone.id)
                .await
                .expect("servers")
                .len(),
            1
        );

        let listed = crate::features::dns::tenant_user_see_zones::run(
            &organisation.database,
            &organisation.admin,
        )
        .await
        .expect("their zones");
        assert_eq!(listed.zones.len(), 1);
    }

    #[tokio::test]
    async fn a_member_who_does_not_administer_the_organisation_cannot() {
        let organisation = crate::features::fixtures::organisation().await;
        let refused = super::run(
            &organisation.database,
            &Container::new(),
            &organisation.member,
            &CreateZoneRequest {
                name: "acme.example".into(),
            },
        )
        .await;
        assert!(refused.is_err());
    }

    /// Another organisation is told the zone does not exist, which is the
    /// same thing it is told about a domain nobody has — or it could learn
    /// which domains its rival keeps here by asking.
    #[tokio::test]
    async fn another_organisation_cannot_see_or_touch_the_zone() {
        let organisation = acme_with_a_domain().await;
        let database = &organisation.database;

        let seen = crate::features::dns::tenant_user_see_zone::run(
            database,
            &organisation.rival,
            "acme.example",
        )
        .await;
        assert_eq!(seen.err().as_deref(), Some(zones::NO_SUCH_ZONE));
        let nowhere = crate::features::dns::tenant_user_see_zone::run(
            database,
            &organisation.rival,
            "nobody.example",
        )
        .await;
        assert_eq!(nowhere.err().as_deref(), Some(zones::NO_SUCH_ZONE));

        let added = crate::features::dns::tenant_user_add_record::run(
            database,
            &Container::new(),
            &organisation.rival,
            &AddRecordRequest {
                zone: "acme.example".into(),
                name: "@".into(),
                kind: "A".into(),
                value: "203.0.113.66".into(),
                ..Default::default()
            },
        )
        .await;
        assert!(added.is_err());
        let deleted = crate::features::dns::tenant_user_delete_zone::run(
            database,
            &Container::new(),
            &organisation.rival,
            "acme.example",
        )
        .await;
        assert!(deleted.is_err());
        assert!(zones::find_by_name(database, "acme.example")
            .await
            .expect("read")
            .is_some());

        let theirs =
            crate::features::dns::tenant_user_see_zones::run(database, &organisation.rival)
                .await
                .expect("their zones");
        assert!(theirs.zones.is_empty(), "a rival was shown acme's zone");
    }
}
