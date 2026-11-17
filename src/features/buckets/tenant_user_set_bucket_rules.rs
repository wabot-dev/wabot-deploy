use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::bucket::Bucket;
use crate::domain::entities::service::Service;
use crate::features::buckets::set_bucket_rules_command::SetBucketRules;
use crate::features::buckets::set_bucket_rules_request::SetBucketRulesRequest;
use crate::features::buckets::store_converged_response::StoreConvergedResponse;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    bucket: &Bucket,
    asked: &SetBucketRulesRequest,
) -> Result<StoreConvergedResponse, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }
    SetBucketRules {
        service,
        bucket,
        asked,
    }
    .run(database)
    .await
}
