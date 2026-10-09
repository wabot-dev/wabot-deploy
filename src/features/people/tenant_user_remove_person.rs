use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::infra::accounts;

pub async fn run(database: &SqliteDatabase, account: &Account, person: &str) -> Result<(), String> {
    if !tenant_user::administers(account) {
        return Err("only an administrator of this organisation can do that".into());
    }

    let theirs = accounts::of_scope(database, account.tenant_id.as_deref())
        .await
        .map_err(|error| error.to_string())?;
    if !theirs.iter().any(|found| found.id == person) {
        return Err("no such person here".into());
    }

    accounts::delete(database, person)
        .await
        .map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::fixtures::organisation;

    #[tokio::test]
    async fn an_administrator_removes_one_of_their_own() {
        let here = organisation().await;
        run(&here.database, &here.admin, &here.member.id)
            .await
            .expect("removed");

        assert!(
            accounts::of_scope(&here.database, here.admin.tenant_id.as_deref())
                .await
                .expect("people")
                .iter()
                .all(|person| person.id != here.member.id)
        );
    }

    #[tokio::test]
    async fn nobody_removes_somebody_of_another_organisation() {
        let here = organisation().await;
        let refused = run(&here.database, &here.admin, &here.rival.id)
            .await
            .expect_err("refused");
        assert!(refused.contains("no such person here"), "{refused}");

        assert!(
            accounts::of_scope(&here.database, here.rival.tenant_id.as_deref())
                .await
                .expect("people")
                .iter()
                .any(|person| person.id == here.rival.id)
        );
    }

    #[tokio::test]
    async fn a_member_removes_nobody() {
        let here = organisation().await;
        assert!(run(&here.database, &here.member, &here.admin.id)
            .await
            .is_err());
    }
}
