use wabot::sqlite::SqliteDatabase;

use crate::platform::services::{self, Service};

pub struct Asked<'a> {
    pub project: &'a str,
    pub name: &'a str,
    pub image: &'a str,
}

pub struct CreateService<'a> {
    pub project_id: &'a str,
    pub name: &'a str,
    pub image: &'a str,
    pub env: &'a [(String, String)],
}

impl CreateService<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Service, String> {
        if !crate::network::capability::owns(database).await {
            return Err("this node does not own services of its own".into());
        }

        services::create(database, self.project_id, self.name, self.image, self.env)
            .await
            .map_err(|error| error.to_string())
    }
}
