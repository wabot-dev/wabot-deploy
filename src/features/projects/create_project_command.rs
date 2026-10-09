use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::domain::values::roles::ProjectRole;
use crate::infra::repositories::access;

pub struct CreateProject<'a> {
    pub name: &'a str,
    pub by: &'a Account,
}

impl CreateProject<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Project, String> {
        // Here rather than in `crate::infra::repositories::projects::create`: the collector calls
        // that too, for a project arriving on an errand, and a node that
        // runs other people's work is exactly the node this setting is
        // for. Refusing there would refuse the thing it exists to allow.
        if !crate::infra::network::capability::owns(database).await {
            return Err(
                "this node does not own projects of its own — turn that back on \
                        from its capabilities if it should"
                    .into(),
            );
        }

        // Inside the frontier of whoever made it. It was always the node's,
        // so an organisation's projects escaped its quota, its count and
        // its deletion — found on a VM node, where deleting an organisation
        // left the project its owner had made.
        let project = crate::infra::repositories::projects::create_for(
            database,
            self.name,
            self.by.tenant_id.as_deref(),
        )
        .await
        .map_err(|error| error.to_string())?;

        // Whoever made it owns it. Without this a member would create a
        // project and immediately not be able to see it, since the list
        // is filtered by membership.
        access::grant(database, &self.by.id, &project.id, ProjectRole::Owner)
            .await
            .map_err(|error| error.to_string())?;

        Ok(project)
    }
}
