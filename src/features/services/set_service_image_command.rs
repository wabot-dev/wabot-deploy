use crate::entities::service::Service;
use crate::repositories::services;
use wabot::sqlite::SqliteDatabase;

pub struct SetServiceImage<'a> {
    pub service: &'a Service,
    pub image: &'a str,
}

impl SetServiceImage<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if self.service.kind.is_managed() {
            return Err("the node writes this for a managed database".into());
        }

        services::set_image(database, &self.service.id, self.image)
            .await
            .map_err(|error| error.to_string())
    }
}
