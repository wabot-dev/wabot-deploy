use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::zone_name::ZoneName;
use crate::features::dns::create_zone_command::CreateZone;
use crate::features::dns::create_zone_request::CreateZoneRequest;
use crate::features::dns::zone_created_response::ZoneCreatedResponse;
use crate::infra::repositories::access;

pub async fn run(
    database: &SqliteDatabase,
    container: &Container,
    account: &Account,
    asked: &CreateZoneRequest,
) -> Result<ZoneCreatedResponse, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }
    let (project, allowed) = access::find_project(database, account, &asked.project)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "no such project".to_string())?;
    if !allowed.may_deploy() {
        return Err("you may look at this project, not change it".into());
    }
    let name = ZoneName::parse(&asked.name)?;
    CreateZone {
        name: &name,
        project: &project,
    }
    .run(database, container)
    .await
}

#[cfg(test)]
mod tests {
    use crate::domain::values::roles::{NodeRole, ProjectRole};
    use crate::features::dns::create_zone_request::CreateZoneRequest;
    use crate::infra::config::Config;
    use crate::infra::repositories::{access, projects, zones};
    use wabot::prelude::Container;

    /// A zone is the project's like a service is, so a deployer of one of
    /// the machine's projects makes one without being the machine's
    /// operator — and who answers for it stays the operator's to choose.
    #[tokio::test]
    async fn a_deployer_makes_a_zone_and_the_operator_chooses_who_answers() {
        let organisation = crate::features::fixtures::organisation().await;
        let database = &organisation.database;
        crate::infra::network::ensure_self(database, &Config::default())
            .await
            .expect("this node");
        let deployer = crate::infra::accounts::create_in(
            database,
            "deployer",
            "a long passphrase here",
            NodeRole::Member,
            None,
        )
        .await
        .expect("a member of this node");
        let site = projects::create(database, "Site").await.expect("project");
        access::grant(database, &deployer.id, &site.id, ProjectRole::Deployer)
            .await
            .expect("deployer");

        super::run(
            database,
            &Container::new(),
            &deployer,
            &CreateZoneRequest {
                project: site.slug.clone(),
                name: "teresaasprilla.com".into(),
            },
        )
        .await
        .expect("the deployer's zone");
        let zone = zones::find_by_name(database, "teresaasprilla.com")
            .await
            .expect("read")
            .expect("there");
        assert_eq!(zone.project_id.as_deref(), Some(site.id.as_str()));

        let theirs = crate::features::dns::node_user_see_zone::run(
            database,
            &deployer,
            &site.slug,
            "teresaasprilla.com",
        )
        .await
        .expect("the deployer's view");
        assert!(theirs.editable);
        assert!(theirs.servers.is_empty(), "a deployer was offered servers");
        let operators = crate::features::dns::node_user_see_zone::run(
            database,
            &organisation.operator,
            &site.slug,
            "teresaasprilla.com",
        )
        .await
        .expect("the operator's view");
        assert!(!operators.servers.is_empty());
    }

    #[tokio::test]
    async fn somebody_with_no_place_in_the_project_makes_nothing_in_it() {
        let organisation = crate::features::fixtures::organisation().await;
        let database = &organisation.database;
        crate::infra::network::ensure_self(database, &Config::default())
            .await
            .expect("this node");
        let outsider = crate::infra::accounts::create_in(
            database,
            "outsider",
            "a long passphrase here",
            NodeRole::Member,
            None,
        )
        .await
        .expect("a member of this node");
        let site = projects::create(database, "Site").await.expect("project");

        let refused = super::run(
            database,
            &Container::new(),
            &outsider,
            &CreateZoneRequest {
                project: site.slug.clone(),
                name: "teresaasprilla.com".into(),
            },
        )
        .await;
        assert_eq!(refused.err().as_deref(), Some("no such project"));
        assert!(zones::all(database).await.expect("zones").is_empty());
    }
}
