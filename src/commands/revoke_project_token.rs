use wabot::sqlite::SqliteDatabase;

use crate::repositories::tokens;

pub struct RevokeProjectToken<'a> {
    pub project_id: &'a str,
    pub token_id: &'a str,
}

impl RevokeProjectToken<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        // Only this project's tokens: an id from another page must not
        // revoke somebody else's.
        let owned = tokens::of_project(database, self.project_id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .any(|token| token.id == self.token_id);
        if !owned {
            return Err("no such token here".into());
        }

        tokens::revoke(database, self.token_id)
            .await
            .map_err(|error| error.to_string())
    }
}
