use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::entities::tenant::Tenant;
use crate::repositories::tenants;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    name: &str,
) -> Result<Tenant, String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    tenants::create(database, name)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecases::fixtures::organisation;

    #[tokio::test]
    async fn an_organisations_administrator_cannot_make_another_organisation() {
        let here = organisation().await;
        assert!(run(&here.database, &here.admin, "Initech").await.is_err());
    }

    #[tokio::test]
    async fn the_operator_makes_one() {
        let here = organisation().await;
        let made = run(&here.database, &here.operator, "Initech")
            .await
            .expect("made");
        assert_eq!(made.name, "Initech");
    }
}
