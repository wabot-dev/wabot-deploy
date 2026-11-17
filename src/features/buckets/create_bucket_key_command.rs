use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::bucket_key::{Access, BucketKey};
use crate::domain::entities::service::Service;
use crate::features::buckets::bucket_key_created_response::BucketKeyCreatedResponse;
use crate::features::buckets::create_bucket_command::{DERIVED, NOT_A_STORE};
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
        if !self.service.is_ours() {
            return Err(DERIVED.into());
        }
        let name = self.asked.name.trim();
        if name.is_empty() {
            return Err("a key needs a name, so it can be told apart when it is revoked".into());
        }
        if crate::infra::deploy::stores::held_elsewhere(database, &self.service.id)
            .await
            .is_some()
        {
            return self.made_here(database, name).await;
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

    /// A store on another node has no Garage here to mint with, and an
    /// answer from that node arrives a collection later, after the page that
    /// had to show the secret. So the key is made here in the shape Garage
    /// mints, and that node imports it as a key coming back.
    async fn made_here(
        &self,
        database: &SqliteDatabase,
        name: &str,
    ) -> Result<BucketKeyCreatedResponse, String> {
        let key = BucketKey {
            id: format!("bk-{}", wabot::prelude::password::generate(12)),
            bucket_id: self.bucket.id.clone(),
            name: name.to_string(),
            access_key_id: format!("GK{}", &hex_of_random()[..24]),
            secret_key: hex_of_random(),
            access: Access::parse(&self.asked.access),
        };
        object_stores::create_key(database, &key)
            .await
            .map_err(|error| error.to_string())?;
        if let Err(reason) = crate::infra::deploy::stores::apply(database, self.service).await {
            let _ = object_stores::delete_key(database, &key.id).await;
            return Err(reason);
        }
        Ok(BucketKeyCreatedResponse {
            id: key.id,
            access_key_id: key.access_key_id,
            secret_key: key.secret_key,
        })
    }
}

fn hex_of_random() -> String {
    use sha2::Digest;
    sha2::Sha256::digest(wabot::prelude::password::generate(48).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::buckets::create_bucket_command::CreateBucket;
    use crate::features::buckets::create_bucket_request::CreateBucketRequest;

    /// The secret is shown once, on the page this request lands on, and
    /// the node keeping the store answers a collection later. Garage takes
    /// a key it did not mint only in its own shape.
    #[tokio::test]
    async fn a_key_for_a_store_elsewhere_is_made_here_in_the_shape_garage_imports() {
        let (database, service) =
            crate::features::fixtures::store_elsewhere(1024 * 1024 * 1024).await;
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
        let made = CreateBucketKey {
            service: &service,
            bucket: &bucket,
            asked: &CreateBucketKeyRequest {
                name: "web".into(),
                access: "read_only".into(),
            },
        }
        .run(&database)
        .await
        .expect("made");
        let hex = |text: &str| text.chars().all(|c| c.is_ascii_hexdigit());
        assert!(
            made.access_key_id.starts_with("GK"),
            "{}",
            made.access_key_id
        );
        assert_eq!(made.access_key_id.len(), 26);
        assert!(hex(&made.access_key_id[2..]));
        assert_eq!(made.secret_key.len(), 64);
        assert!(hex(&made.secret_key));

        let told = crate::features::fixtures::told_far(&database).await;
        assert_eq!(told.keys.len(), 1);
        assert_eq!(told.keys[0].access_key_id, made.access_key_id);
        assert_eq!(told.keys[0].bucket, "uploads");
        assert_eq!(told.keys[0].access, "read_only");
    }

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
