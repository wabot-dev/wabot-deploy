use wabot::sqlite::SqliteDatabase;

use crate::platform::tokens;

pub struct CreateProjectToken<'a> {
    pub project_id: &'a str,
    pub name: &'a str,
    pub by: &'a str,
}

impl CreateProjectToken<'_> {
    /// The secret comes back because it exists nowhere else: it is
    /// stored hashed, so this is the only moment any page can show it.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<String, String> {
        let (_, secret) = tokens::create(database, self.project_id, self.name, self.by)
            .await
            .map_err(|error| error.to_string())?;
        Ok(secret)
    }
}
