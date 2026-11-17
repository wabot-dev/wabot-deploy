use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::service::Service;
use crate::domain::policies::garage;
use crate::domain::values::hostname::Hostname;
use crate::features::buckets::create_bucket_command::{DERIVED, NOT_A_STORE};
use crate::features::buckets::set_bucket_public_request::SetBucketPublicRequest;
use crate::features::buckets::store_converged_response::StoreConvergedResponse;
use crate::features::services::add_service_port_command::AddServicePort;
use crate::features::services::add_service_port_request::AddServicePortRequest;
use crate::features::services::delete_service_port_command::DeleteServicePort;
use crate::infra::repositories::{object_stores, ports};

pub const NO_STORE_NAME: &str = "the store has no name of its own to derive one under, so the \
                                 public name has to be typed";

pub struct SetBucketPublic<'a> {
    pub service: &'a Service,
    pub bucket: &'a Bucket,
    pub asked: &'a SetBucketPublicRequest,
}

impl SetBucketPublic<'_> {
    pub async fn run(&self, database: &SqliteDatabase) -> Result<StoreConvergedResponse, String> {
        if !self.service.kind.is_store() || self.bucket.service_id != self.service.id {
            return Err(NOT_A_STORE.into());
        }
        if !self.service.is_ours() {
            return Err(DERIVED.into());
        }
        match self.asked.public {
            true => self.open(database).await?,
            false => self.close(database).await?,
        }
        Ok(StoreConvergedResponse::of(
            crate::infra::deploy::stores::apply(database, self.service).await,
        ))
    }

    /// The name is routed before the row names it, so a name DNS or another
    /// service refuses leaves the bucket as private as it was.
    async fn open(&self, database: &SqliteDatabase) -> Result<(), String> {
        if let Some(hostname) = &self.bucket.public_hostname {
            return Err(format!(
                "it is read at {hostname} already — make it private first to give it another name"
            ));
        }
        let hostname = match self.asked.hostname.trim() {
            "" => self.derived(database).await?,
            typed => Hostname::parse(typed).as_str().to_string(),
        };
        if !garage::is_alias(&hostname) {
            return Err(garage::NOT_AN_ALIAS.into());
        }
        AddServicePort {
            service: self.service,
            asked: &AddServicePortRequest {
                container_port: garage::WEB_PORT,
                publish: false,
                hostname: hostname.clone(),
                https: true,
                without_authority: false,
            },
        }
        .run(database)
        .await?;
        object_stores::set_public_hostname(database, &self.bucket.id, Some(&hostname))
            .await
            .map_err(|error| error.to_string())
    }

    /// The name goes first and the row after it: a bucket that says it is
    /// private while its name still answers would be the row lying.
    async fn close(&self, database: &SqliteDatabase) -> Result<(), String> {
        let Some(hostname) = &self.bucket.public_hostname else {
            return Ok(());
        };
        if let Some(port) = public_port(database, self.service, hostname).await? {
            DeleteServicePort {
                service: self.service,
                port_id: &port,
            }
            .run(database)
            .await?;
        }
        object_stores::set_public_hostname(database, &self.bucket.id, None)
            .await
            .map_err(|error| error.to_string())
    }

    async fn derived(&self, database: &SqliteDatabase) -> Result<String, String> {
        let store = ports::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|port| port.container_port == garage::S3_PORT)
            .and_then(|port| port.hostname)
            .ok_or(NO_STORE_NAME)?;
        Ok(format!("{}.{store}", self.bucket.name))
    }
}

/// The port that routes a public bucket's name, found by the name: a store
/// has one on the website port for each public bucket.
pub async fn public_port(
    database: &SqliteDatabase,
    service: &Service,
    hostname: &str,
) -> Result<Option<String>, String> {
    Ok(ports::of_service(database, &service.id)
        .await
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|port| {
            port.container_port == garage::WEB_PORT && port.hostname.as_deref() == Some(hostname)
        })
        .map(|port| port.id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::buckets::create_bucket_command::CreateBucket;
    use crate::features::buckets::create_bucket_request::CreateBucketRequest;

    async fn uploads() -> (SqliteDatabase, Service, Bucket) {
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
        (database, service, bucket)
    }

    #[tokio::test]
    async fn a_public_name_garage_could_not_hold_is_refused_before_anything_is_routed() {
        let (database, service, bucket) = uploads().await;
        let refused = SetBucketPublic {
            service: &service,
            bucket: &bucket,
            asked: &SetBucketPublicRequest {
                public: true,
                hostname: format!("{}.example.com", "a".repeat(60)),
            },
        }
        .run(&database)
        .await;
        assert_eq!(refused.err().as_deref(), Some(garage::NOT_AN_ALIAS));
        assert!(ports::of_service(&database, &service.id)
            .await
            .expect("ports")
            .is_empty());
        let row = object_stores::bucket(&database, &service.id, "uploads")
            .await
            .expect("read")
            .expect("bucket");
        assert_eq!(row.public_hostname, None);
    }

    #[tokio::test]
    async fn a_store_with_no_name_of_its_own_asks_for_the_public_one() {
        let (database, service, bucket) = uploads().await;
        let refused = SetBucketPublic {
            service: &service,
            bucket: &bucket,
            asked: &SetBucketPublicRequest {
                public: true,
                hostname: String::new(),
            },
        }
        .run(&database)
        .await;
        assert_eq!(refused.err().as_deref(), Some(NO_STORE_NAME));
    }

    /// Two public buckets of one store are two names on one container port,
    /// which the port table refused until migration 0073.
    #[tokio::test]
    async fn two_names_answer_on_one_port_and_a_plain_port_is_still_declared_once() {
        let (database, service, _) = uploads().await;
        for name in ["a.files.example.com", "b.files.example.com"] {
            ports::create(
                &database,
                &service.id,
                garage::WEB_PORT,
                false,
                Some(name),
                &[],
            )
            .await
            .expect("a second name on the same port");
        }
        ports::create(&database, &service.id, 8080, false, None, &[])
            .await
            .expect("plain");
        let again = ports::create(&database, &service.id, 8080, false, None, &[]).await;
        assert!(again.is_err(), "a plain port declared twice");
    }

    #[tokio::test]
    async fn making_a_public_bucket_private_takes_its_name_away_and_then_the_row() {
        let (database, service, bucket) = uploads().await;
        let hostname = "uploads.files.example.com";
        ports::create(
            &database,
            &service.id,
            garage::WEB_PORT,
            false,
            Some(hostname),
            &[],
        )
        .await
        .expect("port");
        object_stores::set_public_hostname(&database, &bucket.id, Some(hostname))
            .await
            .expect("public");
        let public = object_stores::bucket(&database, &service.id, "uploads")
            .await
            .expect("read")
            .expect("bucket");

        SetBucketPublic {
            service: &service,
            bucket: &public,
            asked: &SetBucketPublicRequest {
                public: false,
                hostname: String::new(),
            },
        }
        .run(&database)
        .await
        .expect("private");

        assert_eq!(public_port(&database, &service, hostname).await, Ok(None));
        let row = object_stores::bucket(&database, &service.id, "uploads")
            .await
            .expect("read")
            .expect("bucket");
        assert_eq!(row.public_hostname, None);
    }
}
