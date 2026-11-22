use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::features::people::sign_out_everywhere_command::SignOutEverywhere;
use crate::features::people::sign_out_everywhere_request::SignOutEverywhereRequest;
use crate::infra::accounts;

/// The people an organisation's administrator may let in or shut out:
/// those they could remove.
pub(crate) async fn reachable(
    database: &SqliteDatabase,
    account: &Account,
    person: &str,
) -> Result<Account, String> {
    if !tenant_user::administers(account) {
        return Err("only an administrator of this organisation can do that".into());
    }
    if person == account.id {
        return Err("that is you — your own way in is a device you trust".into());
    }
    accounts::find(database, person)
        .await
        .map_err(|error| error.to_string())?
        .filter(|found| found.tenant_id.is_some() && found.tenant_id == account.tenant_id)
        .ok_or_else(|| "no such person here".into())
}

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: &SignOutEverywhereRequest,
) -> Result<(), String> {
    let person = reachable(database, account, &asked.person).await?;
    SignOutEverywhere {
        by: account,
        person: &person,
    }
    .run(database)
    .await
}
