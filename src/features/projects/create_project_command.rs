use wabot::sqlite::SqliteDatabase;

use crate::entities::account::Account;
use crate::entities::project::Project;
use crate::repositories::access;
use crate::values::roles::ProjectRole;

pub struct CreateProject<'a> {
    pub name: &'a str,
    pub by: &'a Account,
}

impl CreateProject<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Project, String> {
        // Here rather than in `crate::repositories::projects::create`: the collector calls
        // that too, for a project arriving on an errand, and a node that
        // runs other people's work is exactly the node this setting is
        // for. Refusing there would refuse the thing it exists to allow.
        if !crate::network::capability::owns(database).await {
            return Err(
                "this node does not own projects of its own — turn that back on \
                        from its capabilities if it should"
                    .into(),
            );
        }

        let project = crate::repositories::projects::create(database, self.name)
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
