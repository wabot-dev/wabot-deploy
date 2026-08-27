use wabot::sqlite::SqliteDatabase;

use crate::deploy::Deployer;
use crate::entities::project::Project;
use crate::entities::service::Service;
use crate::repositories::replicas;

/// Throw off a service another node placed here.
///
/// **Evicted, not deleted**: the rows stay so the owner learns to stop
/// asking, which is what the report is for. A service created here has
/// no origin and is deleted instead.
pub struct EvictService<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
}

impl EvictService<'_> {
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        let Some(origin) = self.service.origin_node_id.clone() else {
            return Err(
                "this service was created here — deleting it is the button for that".into(),
            );
        };

        deployer.tear_down(self.project, self.service).await;

        for replica in replicas::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|replica| replica.is_here())
        {
            replicas::evict(database, &replica.id)
                .await
                .map_err(|error| error.to_string())?;
        }

        tracing::info!(service = %self.service.slug, %origin, "evicted a service placed from elsewhere");
        Ok(())
    }
}
