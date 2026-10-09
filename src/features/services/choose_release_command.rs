use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::release::Release;
use crate::domain::entities::service::Service;
use crate::infra::repositories::releases;

pub struct ChooseRelease<'a> {
    pub service: &'a Service,
    pub release_id: &'a str,
}

impl ChooseRelease<'_> {
    /// Only this service's releases: a release id names an image digest,
    /// and one from another service would deploy somebody else's bytes.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Release, String> {
        let Some(release) = releases::find(database, self.release_id)
            .await
            .map_err(|error| error.to_string())?
        else {
            return Err("no such release".into());
        };
        if release.service_id != self.service.id {
            return Err("that release belongs to another service".into());
        }
        Ok(release)
    }
}
