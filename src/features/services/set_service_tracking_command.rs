use crate::entities::service::Service;
use crate::repositories::services;
use wabot::sqlite::SqliteDatabase;

pub struct SetServiceTracking<'a> {
    pub service: &'a Service,
    pub tag: &'a str,
    pub auto_deploy: bool,
}

impl SetServiceTracking<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if self.service.kind.is_managed() {
            return Err("the node writes this for a managed database".into());
        }

        services::set_tracking(
            database,
            &self.service.id,
            (!self.tag.is_empty()).then_some(self.tag),
            self.auto_deploy,
        )
        .await
        .map_err(|error| error.to_string())
    }
}
