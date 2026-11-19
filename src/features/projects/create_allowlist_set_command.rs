use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::values::allowlist::Allowlist;
use crate::domain::values::slug::Slug;
use crate::infra::repositories::allowlist_sets;

pub struct CreateAllowlistSet<'a> {
    pub project: &'a Project,
    pub name: &'a Slug,
    pub list: &'a Allowlist,
}

impl CreateAllowlistSet<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        if self.name.is_empty() {
            return Err("a list needs a name with a letter or a digit in it".into());
        }
        allowlist_sets::create(database, &self.project.id, self.name.as_str(), self.list)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
