use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
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
    if !tenant_user::belongs_here(account) {
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
    use crate::domain::entities::project::Project;
    use crate::domain::values::roles::ProjectRole;
    use crate::features::dns::add_record_request::AddRecordRequest;
    use crate::features::dns::create_zone_request::CreateZoneRequest;
    use crate::infra::config::Config;
    use crate::infra::repositories::{access, projects, zones};
    use wabot::prelude::Container;

    async fn acme_site(
        organisation: &crate::features::fixtures::Organisation,
        member: Option<ProjectRole>,
    ) -> Project {
        crate::infra::network::ensure_self(&organisation.database, &Config::default())
            .await
            .expect("this node");
        let project = projects::create_for(
            &organisation.database,
            "Acme site",
            organisation.admin.tenant_id.as_deref(),
        )
        .await
        .expect("acme's project");
        if let Some(role) = member {
            access::grant(
                &organisation.database,
                &organisation.member.id,
                &project.id,
                role,
            )
            .await
            .expect("the member's place in it");
        }
        project
    }

    fn acme_example(project: &Project) -> CreateZoneRequest {
        CreateZoneRequest {
            project: project.slug.clone(),
            name: "acme.example".into(),
        }
    }

    #[tokio::test]
    async fn a_deployer_adds_a_zone_to_their_organisations_project() {
        let organisation = crate::features::fixtures::organisation().await;
        let site = acme_site(&organisation, Some(ProjectRole::Deployer)).await;
        super::run(
            &organisation.database,
            &Container::new(),
            &organisation.member,
            &acme_example(&site),
        )
        .await
        .expect("a deployer's zone");

        let zone = zones::find_by_name(&organisation.database, "acme.example")
            .await
            .expect("read")
            .expect("there");
        assert_eq!(zone.project_id.as_deref(), Some(site.id.as_str()));
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
            &organisation.member,
            &site.slug,
        )
        .await
        .expect("the project's zones");
        assert_eq!(listed.zones.len(), 1);
        assert!(listed.editable);
    }

    /// A viewer is told everything about the zone and can change none of
    /// it — the page hides the forms, and these are the refusals behind them.
    #[tokio::test]
    async fn a_viewer_reads_the_zones_and_changes_none() {
        let organisation = crate::features::fixtures::organisation().await;
        let site = acme_site(&organisation, Some(ProjectRole::Viewer)).await;
        let database = &organisation.database;
        let viewer = &organisation.member;

        let refused = super::run(database, &Container::new(), viewer, &acme_example(&site)).await;
        assert!(refused.is_err(), "a viewer made a zone");
        super::run(
            database,
            &Container::new(),
            &organisation.admin,
            &acme_example(&site),
        )
        .await
        .expect("the administrator's zone");

        let listed = crate::features::dns::tenant_user_see_zones::run(database, viewer, &site.slug)
            .await
            .expect("a viewer reads the list");
        assert_eq!(listed.zones.len(), 1);
        assert!(!listed.editable);
        let seen = crate::features::dns::tenant_user_see_zone::run(
            database,
            viewer,
            &site.slug,
            "acme.example",
        )
        .await
        .expect("a viewer reads the zone");
        assert!(!seen.editable);
        assert!(
            seen.servers.is_empty(),
            "an organisation chooses no servers"
        );

        let added = crate::features::dns::tenant_user_add_record::run(
            database,
            &Container::new(),
            viewer,
            &AddRecordRequest {
                project: site.slug.clone(),
                zone: "acme.example".into(),
                name: "@".into(),
                kind: "A".into(),
                value: "203.0.113.66".into(),
                ..Default::default()
            },
        )
        .await;
        assert!(added.is_err(), "a viewer added a record");
        let deleted = crate::features::dns::tenant_user_delete_zone::run(
            database,
            &Container::new(),
            viewer,
            &site.slug,
            "acme.example",
        )
        .await;
        assert!(deleted.is_err(), "a viewer deleted the zone");
        assert!(zones::find_by_name(database, "acme.example")
            .await
            .expect("read")
            .is_some());
    }

    /// Another organisation is told the project does not exist, which is
    /// what it is told about a project nobody has — or it could learn which
    /// domains its rival keeps here by asking.
    #[tokio::test]
    async fn another_organisation_cannot_see_or_touch_the_zone() {
        let organisation = crate::features::fixtures::organisation().await;
        let site = acme_site(&organisation, None).await;
        let database = &organisation.database;
        super::run(
            database,
            &Container::new(),
            &organisation.admin,
            &acme_example(&site),
        )
        .await
        .expect("acme's zone");

        let seen = crate::features::dns::tenant_user_see_zone::run(
            database,
            &organisation.rival,
            &site.slug,
            "acme.example",
        )
        .await;
        let nowhere = crate::features::dns::tenant_user_see_zone::run(
            database,
            &organisation.rival,
            "nobody",
            "acme.example",
        )
        .await;
        assert_eq!(seen.err(), nowhere.err());

        let added = crate::features::dns::tenant_user_add_record::run(
            database,
            &Container::new(),
            &organisation.rival,
            &AddRecordRequest {
                project: site.slug.clone(),
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
            &site.slug,
            "acme.example",
        )
        .await;
        assert!(deleted.is_err());
        assert!(zones::find_by_name(database, "acme.example")
            .await
            .expect("read")
            .is_some());
        assert!(crate::features::dns::tenant_user_see_zones::run(
            database,
            &organisation.rival,
            &site.slug
        )
        .await
        .is_err());
    }

    /// A member of an organisation is in some of its projects and not
    /// others, and a zone is the project's, not the organisation's.
    #[tokio::test]
    async fn a_member_does_not_reach_the_zones_of_a_project_they_are_not_in() {
        let organisation = crate::features::fixtures::organisation().await;
        let site = acme_site(&organisation, None).await;
        super::run(
            &organisation.database,
            &Container::new(),
            &organisation.admin,
            &acme_example(&site),
        )
        .await
        .expect("acme's zone");

        let seen = crate::features::dns::tenant_user_see_zone::run(
            &organisation.database,
            &organisation.member,
            &site.slug,
            "acme.example",
        )
        .await;
        assert_eq!(seen.err().as_deref(), Some("no such project"));
    }
}
