use wabot::sqlite::SqliteDatabase;

use crate::repositories::access;

pub struct RemoveProjectMember<'a> {
    pub project_id: &'a str,
    pub account_id: &'a str,
}

impl RemoveProjectMember<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        access::revoke(database, self.account_id, self.project_id)
            .await
            .map_err(|error| error.to_string())
    }
}
