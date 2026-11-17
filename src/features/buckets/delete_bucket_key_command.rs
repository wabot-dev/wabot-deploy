use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket_key::BucketKey;
use crate::domain::entities::service::Service;
use crate::features::buckets::create_bucket_command::{DERIVED, NOT_A_STORE};
use crate::infra::repositories::object_stores;

pub struct DeleteBucketKey<'a> {
    pub service: &'a Service,
    pub key: &'a BucketKey,
}

impl DeleteBucketKey<'_> {
    /// Out of Garage first, then off the page: a key the console revoked has
    /// to stop working whether or not anything after this finishes. A store
    /// that is not running holds nobody's request, and the convergence at its
    /// next start deletes every key no row names.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if !self.service.kind.is_store() {
            return Err(NOT_A_STORE.into());
        }
        if !self.service.is_ours() {
            return Err(DERIVED.into());
        }
        let in_store = object_stores::keys_of_store(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .iter()
            .any(|key| key.id == self.key.id);
        if !in_store {
            return Err(NOT_A_STORE.into());
        }
        if let Ok(admin) = crate::infra::garage::admin_for(database, &self.service.id).await {
            admin.delete_key(&self.key.access_key_id).await?;
        }
        object_stores::delete_key(database, &self.key.id)
            .await
            .map_err(|error| error.to_string())?;
        crate::infra::deploy::stores::apply(database, self.service).await?;
        Ok(())
    }
}
