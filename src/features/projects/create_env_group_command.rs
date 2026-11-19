use std::collections::BTreeMap;

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::values::slug::Slug;
use crate::infra::repositories::env_groups;

pub struct CreateEnvGroup<'a> {
    pub project: &'a Project,
    pub name: &'a Slug,
    pub env: &'a BTreeMap<String, String>,
}

impl CreateEnvGroup<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        if self.name.is_empty() {
            return Err("a group needs a name with a letter or a digit in it".into());
        }
        env_groups::create(database, &self.project.id, self.name.as_str(), self.env)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}
