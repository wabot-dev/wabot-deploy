use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::network;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    enrolment: &str,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    network::enrolment::withdraw(database, enrolment)
        .await
        .map_err(|error| error.to_string())
}
