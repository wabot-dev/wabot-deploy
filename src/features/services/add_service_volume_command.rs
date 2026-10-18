use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::domain::entities::volume::Volume;
use crate::infra::repositories::{replicas, volumes};

pub struct AddServiceVolume<'a> {
    pub service: &'a Service,
    pub asked: &'a crate::features::services::add_service_volume_request::AddServiceVolumeRequest,
}

impl AddServiceVolume<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Volume, String> {
        if self.service.kind.is_managed() {
            return Err("a database's storage is the node's to declare".into());
        }
        // The errand that places a copy elsewhere carries no volumes, so
        // that copy would start on an empty directory and look as if it
        // had lost everything.
        let elsewhere = replicas::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .any(|replica| !replica.is_here() && !replica.evicted());
        if elsewhere {
            return Err(
                "a volume is a directory on this machine, and a copy on another node \
                 would start without it — bring the copies here first"
                    .into(),
            );
        }

        let name = match self.asked.name.trim() {
            "" => self
                .asked
                .path
                .trim()
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or_default()
                .to_string(),
            typed => typed.to_string(),
        };
        volumes::create(database, &self.service.id, &name, &self.asked.path)
            .await
            .map_err(|error| error.to_string())
    }
}
