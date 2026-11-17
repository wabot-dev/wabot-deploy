use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::{Bucket, DAYS};
use crate::domain::entities::service::Service;
use crate::domain::values::cors_origin::CorsOrigin;
use crate::features::buckets::create_bucket_command::{DERIVED, NOT_A_STORE};
use crate::features::buckets::set_bucket_rules_request::SetBucketRulesRequest;
use crate::features::buckets::store_converged_response::StoreConvergedResponse;
use crate::infra::repositories::object_stores;

pub struct SetBucketRules<'a> {
    pub service: &'a Service,
    pub bucket: &'a Bucket,
    pub asked: &'a SetBucketRulesRequest,
}

impl SetBucketRules<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<StoreConvergedResponse, String> {
        if !self.service.kind.is_store() || self.bucket.service_id != self.service.id {
            return Err(NOT_A_STORE.into());
        }
        if !self.service.is_ours() {
            return Err(DERIVED.into());
        }
        let max_size = crate::domain::policies::presets::parse_bytes(&self.asked.max_size)?
            .ok_or(crate::domain::entities::bucket::NO_ROOM)?;
        let mut bucket = self.bucket.clone();
        bucket.cors_origins = CorsOrigin::parse_lines(&self.asked.cors_origins)?;
        bucket.set_expiry(
            days(&self.asked.expire_days)?,
            days(&self.asked.abort_uploads_days)?,
        )?;
        if max_size != self.bucket.max_size {
            object_stores::set_max_size(database, &bucket, max_size)
                .await
                .map_err(|error| error.to_string())?;
        }
        object_stores::set_access_rules(database, &bucket)
            .await
            .map_err(|error| error.to_string())?;
        Ok(StoreConvergedResponse::of(
            crate::infra::deploy::stores::apply(database, self.service).await,
        ))
    }
}

fn days(typed: &str) -> Result<Option<u32>, &'static str> {
    match typed.trim() {
        "" => Ok(None),
        text => text.parse().map(Some).map_err(|_| DAYS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::buckets::create_bucket_command::CreateBucket;
    use crate::features::buckets::create_bucket_request::CreateBucketRequest;

    const GB: u64 = 1024 * 1024 * 1024;

    async fn uploads(size: &str) -> (SqliteDatabase, Service, Bucket) {
        let (database, service) = crate::features::fixtures::store_of_buckets(10 * GB).await;
        let (bucket, _) = CreateBucket {
            service: &service,
            asked: &CreateBucketRequest {
                name: "uploads".into(),
                max_size: size.into(),
            },
        }
        .run(&database)
        .await
        .expect("bucket");
        (database, service, bucket)
    }

    fn rules(max_size: &str) -> SetBucketRulesRequest {
        SetBucketRulesRequest {
            max_size: max_size.into(),
            cors_origins: "https://app.example.com\nhttps://*.example.com".into(),
            expire_days: "30".into(),
            abort_uploads_days: String::new(),
        }
    }

    #[tokio::test]
    async fn the_rules_are_saved_as_typed_and_read_back() {
        let (database, service, bucket) = uploads("1 GB").await;
        SetBucketRules {
            service: &service,
            bucket: &bucket,
            asked: &rules("2 GB"),
        }
        .run(&database)
        .await
        .expect("saved");
        let read = object_stores::bucket(&database, &service.id, "uploads")
            .await
            .expect("read")
            .expect("there");
        assert_eq!(read.max_size, 2 * GB);
        assert_eq!(read.cors_origins.len(), 2);
        assert_eq!(
            (read.expire_days, read.abort_uploads_days),
            (Some(30), None)
        );
    }

    #[tokio::test]
    async fn growing_a_bucket_past_its_store_is_refused_and_nothing_is_saved() {
        let (database, service, bucket) = uploads("1 GB").await;
        let refused = SetBucketRules {
            service: &service,
            bucket: &bucket,
            asked: &rules("11 GB"),
        }
        .run(&database)
        .await
        .expect_err("past the ceiling");
        assert!(refused.contains("10 GB"), "{refused}");
        let read = object_stores::bucket(&database, &service.id, "uploads")
            .await
            .expect("read")
            .expect("there");
        assert_eq!(read, bucket);
    }

    #[tokio::test]
    async fn zero_or_words_for_days_are_refused_with_what_days_are() {
        let (database, service, bucket) = uploads("1 GB").await;
        for typed in ["0", "a week", "-3"] {
            let refused = SetBucketRules {
                service: &service,
                bucket: &bucket,
                asked: &SetBucketRulesRequest {
                    expire_days: typed.into(),
                    ..rules("1 GB")
                },
            }
            .run(&database)
            .await
            .expect_err(typed);
            assert_eq!(refused, DAYS, "{typed}");
        }
    }
}
