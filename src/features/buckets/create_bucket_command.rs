use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::service::Service;
use crate::domain::values::bucket_name::BucketName;
use crate::features::buckets::create_bucket_request::CreateBucketRequest;
use crate::features::buckets::store_converged_response::StoreConvergedResponse;
use crate::infra::repositories::object_stores;

pub const NOT_A_STORE: &str = "that service is not a store of buckets";

pub struct CreateBucket<'a> {
    pub service: &'a Service,
    pub asked: &'a CreateBucketRequest,
}

impl CreateBucket<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
    ) -> Result<(Bucket, StoreConvergedResponse), String> {
        if !self.service.kind.is_store() {
            return Err(NOT_A_STORE.into());
        }
        let name = BucketName::parse(&self.asked.name)?;
        let max_size = crate::domain::policies::presets::parse_bytes(&self.asked.max_size)?
            .ok_or(crate::domain::entities::bucket::NO_ROOM)?;
        let bucket = Bucket::new(
            format!("bkt-{}", wabot::prelude::password::generate(12)),
            self.service.id.clone(),
            name,
            max_size,
        )?;
        object_stores::create_bucket(database, &bucket)
            .await
            .map_err(|error| error.to_string())?;
        let converged = crate::infra::garage::converge_if_running(database, &self.service.id).await;
        Ok((bucket, StoreConvergedResponse::of(converged)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::service::Kind;

    const GB: u64 = 1024 * 1024 * 1024;

    async fn store(disk: u64) -> (SqliteDatabase, Service) {
        crate::features::fixtures::store_of_buckets(disk).await
    }

    async fn make(
        database: &SqliteDatabase,
        service: &Service,
        name: &str,
        max_size: &str,
    ) -> Result<(Bucket, StoreConvergedResponse), String> {
        CreateBucket {
            service,
            asked: &CreateBucketRequest {
                name: name.into(),
                max_size: max_size.into(),
            },
        }
        .run(database)
        .await
    }

    #[tokio::test]
    async fn a_bucket_is_saved_and_says_its_store_is_not_running_to_take_it() {
        let (database, service) = store(10 * GB).await;
        let (bucket, converged) = make(&database, &service, "Uploads", "5 GB")
            .await
            .expect("made");
        assert_eq!(bucket.name.as_str(), "uploads");
        assert_eq!(converged, StoreConvergedResponse::NotRunning);
        assert_eq!(
            object_stores::buckets(&database, &service.id)
                .await
                .expect("buckets"),
            vec![bucket]
        );
    }

    #[tokio::test]
    async fn the_buckets_of_a_store_together_fit_its_disk_and_a_refusal_says_the_numbers() {
        let (database, service) = store(10 * GB).await;
        make(&database, &service, "uploads", "6 GB")
            .await
            .expect("first");
        let refused = make(&database, &service, "avatars", "5 GB")
            .await
            .expect_err("past the ceiling");
        for number in ["5 GB", "10 GB", "6 GB", "4 GB"] {
            assert!(refused.contains(number), "{number} missing from: {refused}");
        }
        make(&database, &service, "avatars", "4 GB")
            .await
            .expect("exactly what is left");
    }

    #[tokio::test]
    async fn a_name_the_store_already_has_or_that_is_not_one_is_refused() {
        let (database, service) = store(10 * GB).await;
        make(&database, &service, "uploads", "1 GB")
            .await
            .expect("first");
        assert_eq!(
            make(&database, &service, "uploads", "1 GB")
                .await
                .expect_err("taken"),
            object_stores::TAKEN
        );
        assert_eq!(
            make(&database, &service, "my.bucket", "1 GB")
                .await
                .expect_err("dotted"),
            crate::domain::values::bucket_name::CHARACTERS
        );
    }

    #[tokio::test]
    async fn a_service_that_is_not_a_store_holds_no_buckets() {
        let (database, service) = store(10 * GB).await;
        let container = Service {
            kind: Kind::Container,
            ..service
        };
        assert_eq!(
            make(&database, &container, "uploads", "1 GB")
                .await
                .expect_err("a container"),
            NOT_A_STORE
        );
    }
}
