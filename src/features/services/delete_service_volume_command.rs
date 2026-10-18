use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::infra::repositories::volumes;

pub struct DeleteServiceVolume<'a> {
    pub service: &'a Service,
    pub volume_id: &'a str,
}

impl DeleteServiceVolume<'_> {
    /// Only this service's, and only the row: what the directory holds
    /// stays until the service is deleted, because that is the one
    /// decision here with no way back.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if self.service.kind.is_managed() {
            return Err("a database's storage is the node's to remove".into());
        }
        let owned = volumes::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .any(|volume| volume.id == self.volume_id);
        if !owned {
            return Err("no such volume here".into());
        }
        volumes::delete(database, self.volume_id)
            .await
            .map_err(|error| error.to_string())
    }
}
