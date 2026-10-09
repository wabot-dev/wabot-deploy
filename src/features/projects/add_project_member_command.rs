use wabot::sqlite::SqliteDatabase;

use crate::domain::values::roles::ProjectRole;
use crate::infra::repositories::access;

pub struct AddProjectMember<'a> {
    pub project_id: &'a str,
    pub username: &'a str,
    pub role: ProjectRole,
}

impl AddProjectMember<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        let Some(found) = crate::infra::accounts::all(database)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|candidate| candidate.username.eq_ignore_ascii_case(self.username))
        else {
            return Err(format!(
                "nobody on this node is called {:?} — invite them first",
                self.username
            ));
        };

        access::grant(database, &found.id, self.project_id, self.role)
            .await
            .map_err(|error| error.to_string())?;
        Ok(())
    }
}
