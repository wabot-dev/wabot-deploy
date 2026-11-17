use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::service::Service;
use crate::features::buckets::delete_bucket_command::DeleteBucket;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    bucket: &Bucket,
) -> Result<(), String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to delete".into());
    }
    DeleteBucket { service, bucket }.run(database).await
}
