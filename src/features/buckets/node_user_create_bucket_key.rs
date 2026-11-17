use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::service::Service;
use crate::features::buckets::bucket_key_created_response::BucketKeyCreatedResponse;
use crate::features::buckets::create_bucket_key_command::CreateBucketKey;
use crate::features::buckets::create_bucket_key_request::CreateBucketKeyRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    bucket: &Bucket,
    asked: &CreateBucketKeyRequest,
) -> Result<BucketKeyCreatedResponse, String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }
    CreateBucketKey {
        service,
        bucket,
        asked,
    }
    .run(database)
    .await
}
