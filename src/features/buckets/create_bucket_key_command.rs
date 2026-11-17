use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::bucket_key::{Access, BucketKey};
use crate::domain::entities::service::Service;
use crate::features::buckets::bucket_key_created_response::BucketKeyCreatedResponse;
use crate::features::buckets::create_bucket_command::NOT_A_STORE;
use crate::features::buckets::create_bucket_key_request::CreateBucketKeyRequest;
use crate::infra::repositories::object_stores;

pub struct CreateBucketKey<'a> {
    pub service: &'a Service,
    pub bucket: &'a Bucket,
    pub asked: &'a CreateBucketKeyRequest,
}

impl CreateBucketKey<'_> {
    /// **A key works the moment the page shows it**, so the running store
    /// mints it, the row is written, and the permissions are converged in the
    /// same request. A failure after Garage minted it takes the key back out
    /// of Garage: a key that exists there and on no page is one nobody can
    /// revoke.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<BucketKeyCreatedResponse, String> {
        if !self.service.kind.is_store() || self.bucket.service_id != self.service.id {
            return Err(NOT_A_STORE.into());
        }
        let name = self.asked.name.trim();
        if name.is_empty() {
            return Err("a key needs a name, so it can be told apart when it is revoked".into());
        }
        let admin = crate::infra::garage::admin_for(database, &self.service.id)
            .await
            .map_err(|reason| format!("a key is minted by the store: {reason}"))?;
        let (access_key_id, secret_key) = admin.mint_key(name).await?;
        let key = BucketKey {
            id: format!("bk-{}", wabot::prelude::password::generate(12)),
            bucket_id: self.bucket.id.clone(),
            name: name.to_string(),
            access_key_id,
            secret_key,
            access: Access::parse(&self.asked.access),
        };
        let kept = match object_stores::create_key(database, &key).await {
            Ok(()) => crate::infra::garage::converge_if_running(database, &self.service.id)
                .await
                .map(|_| ()),
            Err(error) => Err(error.to_string()),
        };
        if let Err(reason) = kept {
            let _ = object_stores::delete_key(database, &key.id).await;
            if let Err(error) = admin.delete_key(&key.access_key_id).await {
                tracing::error!(key = %key.access_key_id, %error, "a refused key is still in the store");
            }
            return Err(reason);
        }
        Ok(BucketKeyCreatedResponse {
            id: key.id,
            access_key_id: key.access_key_id,
            secret_key: key.secret_key,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::buckets::create_bucket_command::CreateBucket;
    use crate::features::buckets::create_bucket_request::CreateBucketRequest;

    #[tokio::test]
    async fn no_key_is_written_that_the_store_did_not_mint() {
        let (database, service) =
            crate::features::fixtures::store_of_buckets(1024 * 1024 * 1024).await;
        let (bucket, _) = CreateBucket {
            service: &service,
            asked: &CreateBucketRequest {
                name: "uploads".into(),
                max_size: "100 MB".into(),
            },
        }
        .run(&database)
        .await
        .expect("bucket");
        let refused = CreateBucketKey {
            service: &service,
            bucket: &bucket,
            asked: &CreateBucketKeyRequest {
                name: "web".into(),
                access: "read_write".into(),
            },
        }
        .run(&database)
        .await
        .expect_err("not running");
        assert!(
            refused.starts_with("a key is minted by the store"),
            "{refused}"
        );
        assert!(object_stores::keys(&database, &bucket.id)
            .await
            .expect("keys")
            .is_empty());
    }
}
