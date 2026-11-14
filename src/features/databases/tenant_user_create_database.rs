use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::features::databases::create_database_request::CreateDatabaseRequest;
use crate::infra::repositories::access;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &CreateDatabaseRequest,
) -> Result<(Project, Service), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }

    let Some((project, allowed)) = access::find_project(database, account, &asked.project)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such project".into());
    };
    if !allowed.may_deploy() {
        return Err("you cannot deploy in this project".into());
    }

    // Blank is not "no ceiling" here. An unbounded database on a node
    // with one core is the node, so the form's empty answer is the
    // form's default answer.
    let memory_limit = crate::domain::policies::presets::parse(&asked.memory)?
        .unwrap_or(crate::features::databases::create_database_command::DEFAULT_PRESET);

    let on = match asked.on.trim() {
        "" => None,
        id => match crate::infra::network::candidates_for(
            database,
            crate::domain::values::capability::Capability::Store,
        )
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|node| node.id == id)
        {
            Some(node) if node.is_self => None,
            Some(node) => Some(node.id),
            None => return Err("that node has not agreed to keep data for this one".into()),
        },
    };

    let (service, _) = crate::features::databases::create_database_command::CreateDatabase {
        project_id: &project.id,
        name: &asked.name,
        engine: crate::domain::entities::database::Engine::parse(&asked.engine),
        version: &asked.version,
        memory_limit,
        image: asked.image.as_deref(),
        on: on.as_deref(),
    }
    .run(database)
    .await
    .map_err(|error| error.to_string())?;

    Ok((project, service))
}

#[cfg(test)]
mod tests {
    use crate::features::databases::create_database_request::CreateDatabaseRequest;
    use crate::infra::repositories::projects;

    #[tokio::test]
    async fn an_organisation_makes_a_database_from_an_image_of_its_own() {
        let organisation = crate::features::fixtures::organisation().await;
        let project = projects::create_for(
            &organisation.database,
            "Acme site",
            organisation.admin.tenant_id.as_deref(),
        )
        .await
        .expect("acme's project");
        let asked = |image: &str| CreateDatabaseRequest {
            project: project.slug.clone(),
            name: "vectors".into(),
            engine: "postgres".into(),
            version: "17".into(),
            memory: String::new(),
            image: Some(image.into()),
            on: String::new(),
        };

        let refused = super::run(
            &organisation.database,
            &organisation.admin,
            &asked("pgvector/pgvector:pg17-trixie"),
        )
        .await
        .expect_err("a reference with no registry");
        assert!(
            refused.contains("docker.io/pgvector/pgvector:pg17-trixie"),
            "{refused}"
        );

        let (_, service) = super::run(
            &organisation.database,
            &organisation.admin,
            &asked("docker.io/pgvector/pgvector:pg17-trixie"),
        )
        .await
        .expect("made");
        assert_eq!(service.image, "docker.io/pgvector/pgvector:pg17-trixie");
    }
}
