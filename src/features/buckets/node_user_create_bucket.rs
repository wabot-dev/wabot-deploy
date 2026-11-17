use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::service::Service;
use crate::features::buckets::create_bucket_command::CreateBucket;
use crate::features::buckets::create_bucket_request::CreateBucketRequest;
use crate::features::buckets::store_converged_response::StoreConvergedResponse;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    asked: &CreateBucketRequest,
) -> Result<(Bucket, StoreConvergedResponse), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }
    CreateBucket { service, asked }.run(database).await
}
