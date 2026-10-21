use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::domain::errors::PlatformError;
use crate::infra::repositories::redirects;

pub struct DeleteServiceRedirect<'a> {
    pub service: &'a Service,
    pub hostname: &'a str,
}

impl DeleteServiceRedirect<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        redirects::delete(database, &self.service.id, self.hostname)
            .await
            .map_err(|error| match error {
                PlatformError::Refused(reason) => reason,
                other => {
                    tracing::warn!(error = %other, "could not delete a redirect");
                    "that could not be saved".to_string()
                }
            })
    }
}
