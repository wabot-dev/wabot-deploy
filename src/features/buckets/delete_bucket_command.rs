use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::service::Service;
use crate::features::buckets::create_bucket_command::{DERIVED, NOT_A_STORE};
use crate::features::buckets::set_bucket_public_command::{own_port, public_port};
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
        if !self.service.is_ours() {
            return Err(DERIVED.into());
        }
        let elsewhere =
            crate::infra::deploy::stores::held_elsewhere(database, &self.service.id).await;
        match &elsewhere {
            Some(holder) => self.reported_empty(database, &holder.name).await?,
            None => self.emptied_here(database).await?,
        }
        let public = match &self.bucket.public_hostname {
            Some(hostname) => public_port(database, self.service, hostname).await?,
            None => None,
        };
        let own = own_port(database, self.service, self.bucket).await?;
        for port in public.iter().chain(own.iter()) {
            crate::features::services::delete_service_port_command::DeleteServicePort {
                service: self.service,
                port_id: port,
            }
            .run(database)
            .await?;
        }
        object_stores::delete_bucket(database, &self.bucket.id)
            .await
            .map_err(|error| error.to_string())?;
        if elsewhere.is_some() {
            crate::infra::deploy::stores::apply(database, self.service).await?;
        }
        Ok(())
    }

    async fn emptied_here(&self, database: &SqliteDatabase) -> Result<(), String> {
        let admin = crate::infra::garage::admin_for(database, &self.service.id)
            .await
            .map_err(|reason| format!("whether it is empty is asked of the store: {reason}"))?;
        if let Some(usage) = admin.usage(self.bucket.name.as_str()).await? {
            refuse_unless_empty(usage.objects, usage.unfinished_uploads)?;
        }
        admin.delete_bucket(self.bucket.name.as_str()).await?;
        for key in object_stores::keys(database, &self.bucket.id)
            .await
            .map_err(|error| error.to_string())?
        {
            admin.delete_key(&key.access_key_id).await?;
        }
        Ok(())
    }

    /// **What that node last reported**, because this one cannot ask its
    /// Garage. The node asks again before it deletes, and Garage itself
    /// refuses to delete a bucket holding an object, so an upload that
    /// arrived after the report keeps the bucket there rather than losing it.
    async fn reported_empty(&self, database: &SqliteDatabase, node: &str) -> Result<(), String> {
        let measured = object_stores::measured(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|measured| measured.name == self.bucket.name.as_str())
            .ok_or_else(|| {
                format!(
                    "whether it is empty is asked of the store, which runs on {node} and has not \
                     reported this bucket yet"
                )
            })?;
        refuse_unless_empty(measured.objects, measured.unfinished_uploads)
    }
}

fn refuse_unless_empty(objects: u64, unfinished_uploads: u64) -> Result<(), String> {
    if objects == 0 && unfinished_uploads == 0 {
        return Ok(());
    }
    Err(format!(
        "{}: this one holds {} and {}",
        object_stores::NOT_EMPTY,
        counted(objects, "object", "objects"),
        counted(
            unfinished_uploads,
            "unfinished upload",
            "unfinished uploads"
        ),
    ))
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

    /// This node cannot ask a Garage on another machine, so it goes by what
    /// that machine last reported — and deletes nothing it has not heard
    /// about, or heard holds something.
    #[tokio::test]
    async fn a_bucket_elsewhere_is_deleted_only_once_its_node_reported_it_empty() {
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
        let delete = || DeleteBucket {
            service: &service,
            bucket: &bucket,
        };

        let unheard = delete().run(&database).await.expect_err("no report");
        assert!(unheard.contains("far.example"), "{unheard}");

        let measured = |objects| crate::domain::entities::bucket::Measured {
            name: "uploads".into(),
            objects,
            bytes: objects * 10,
            unfinished_uploads: 0,
        };
        object_stores::set_measured(&database, &service.id, &[measured(3)])
            .await
            .expect("report");
        let full = delete().run(&database).await.expect_err("not empty");
        assert!(full.starts_with(object_stores::NOT_EMPTY), "{full}");

        object_stores::set_measured(&database, &service.id, &[measured(0)])
            .await
            .expect("report");
        delete().run(&database).await.expect("empty");
        assert!(object_stores::buckets(&database, &service.id)
            .await
            .expect("buckets")
            .is_empty());
        assert!(crate::features::fixtures::told_far(&database)
            .await
            .buckets
            .is_empty());
    }

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
