use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::domain::values::command_line::CommandLine;
use crate::infra::repositories::services;

pub struct SetServiceProcess<'a> {
    pub service: &'a Service,
    pub entrypoint: Option<&'a CommandLine>,
    pub command: Option<&'a CommandLine>,
}

impl SetServiceProcess<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if self.service.kind.is_managed() {
            return Err("the node decides what a managed service runs".into());
        }
        services::set_process(database, &self.service.id, self.entrypoint, self.command)
            .await
            .map_err(|error| error.to_string())
    }
}
