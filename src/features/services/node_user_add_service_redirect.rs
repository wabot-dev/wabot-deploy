use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::service::Service;
use crate::features::services::add_service_redirect_command::AddServiceRedirect;
use crate::features::services::add_service_redirect_request::AddServiceRedirectRequest;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    service: &Service,
    asked: &AddServiceRedirectRequest,
) -> Result<(), String> {
    if !node_user::belongs_here(account) {
        return Err("that is not this console's to change".into());
    }
    AddServiceRedirect { service, asked }.run(database).await
}
