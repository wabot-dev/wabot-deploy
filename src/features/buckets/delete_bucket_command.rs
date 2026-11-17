use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::service::Service;
use crate::features::buckets::create_bucket_command::NOT_A_STORE;
use crate::infra::repositories::object_stores;

pub struct DeleteBucket<'a> {
    pub service: &'a Service,
    pub bucket: &'a Bucket,
}

impl DeleteBucket<'_> {
    /// Asked of the running store, because whether a bucket is empty is
    /// Garage's to know. Emptying a large one is a job rather than a request,
    /// so the refusal says how much is there and leaves the emptying to a
    /// client.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if !self.service.kind.is_store() || self.bucket.service_id != self.service.id {
            return Err(NOT_A_STORE.into());
        }
        let admin = crate::infra::garage::admin_for(database, &self.service.id)
            .await
            .map_err(|reason| format!("whether it is empty is asked of the store: {reason}"))?;
        if let Some(usage) = admin.usage(self.bucket.name.as_str()).await? {
            if usage.objects > 0 || usage.unfinished_uploads > 0 {
                return Err(format!(
                    "{}: this one holds {} and {}",
                    object_stores::NOT_EMPTY,
                    counted(usage.objects, "object", "objects"),
                    counted(
                        usage.unfinished_uploads,
                        "unfinished upload",
                        "unfinished uploads"
                    ),
                ));
            }
        }
        admin.delete_bucket(self.bucket.name.as_str()).await?;
        for key in object_stores::keys(database, &self.bucket.id)
            .await
            .map_err(|error| error.to_string())?
        {
            admin.delete_key(&key.access_key_id).await?;
        }
        if let Some(hostname) = &self.bucket.public_hostname {
            if let Some(port) = crate::features::buckets::set_bucket_public_command::public_port(
                database,
                self.service,
                hostname,
            )
            .await?
            {
                crate::features::services::delete_service_port_command::DeleteServicePort {
                    service: self.service,
                    port_id: &port,
                }
                .run(database)
                .await?;
            }
        }
        object_stores::delete_bucket(database, &self.bucket.id)
            .await
            .map_err(|error| error.to_string())
    }
}

fn counted(n: u64, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        _ => format!("{n} {many}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::buckets::create_bucket_command::CreateBucket;
    use crate::features::buckets::create_bucket_request::CreateBucketRequest;

    /// A bucket's row is the console's record that Garage may hold objects
    /// under that name. Dropping it while nothing could check would hide them.
    #[tokio::test]
    async fn a_bucket_is_not_deleted_while_nothing_can_say_it_is_empty() {
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
        let refused = DeleteBucket {
            service: &service,
            bucket: &bucket,
        }
        .run(&database)
        .await
        .expect_err("not running");
        assert!(refused.contains("not running"), "{refused}");
        assert_eq!(
            object_stores::buckets(&database, &service.id)
                .await
                .expect("buckets"),
            vec![bucket]
        );
    }
}
