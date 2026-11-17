use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::bucket_key::BucketKey;
use crate::domain::entities::service::Service;
use crate::features::buckets::delete_bucket_key_command::DeleteBucketKey;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    key: &BucketKey,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to revoke".into());
    }
    DeleteBucketKey { service, key }.run(database).await
}
