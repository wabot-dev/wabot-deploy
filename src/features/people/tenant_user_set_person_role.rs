use wabot::sqlite::SqliteDatabase;

use crate::accounts;
use crate::authz::tenant_user;
use crate::entities::account::Account;
use crate::values::roles::NodeRole;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    person: &str,
    role: NodeRole,
) -> Result<(), String> {
    if !tenant_user::administers(account) {
        return Err("only an administrator of this organisation can do that".into());
    }

    let theirs = accounts::of_scope(database, account.tenant_id.as_deref())
        .await
        .map_err(|error| error.to_string())?;
    if !theirs.iter().any(|found| found.id == person) {
        return Err("no such person here".into());
    }

    accounts::set_role(database, person, role)
        .await
        .map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::fixtures::organisation;

    #[tokio::test]
    async fn an_administrator_promotes_one_of_their_own() {
        let here = organisation().await;
        run(
            &here.database,
            &here.admin,
            &here.member.id,
            NodeRole::Admin,
        )
        .await
        .expect("promoted");

        let people = accounts::of_scope(&here.database, here.admin.tenant_id.as_deref())
            .await
            .expect("people");
        let found = people
            .iter()
            .find(|person| person.id == here.member.id)
            .expect("still here");
        assert_eq!(found.role, NodeRole::Admin);
    }

    /// An administrator of one organisation reaching into another is the
    /// whole reason the use case looks the person up in its own scope
    /// before it calls the command.
    #[tokio::test]
    async fn nobody_promotes_somebody_of_another_organisation() {
        let here = organisation().await;
        let refused = run(
            &here.database,
            &here.admin,
            &here.rival.id,
            NodeRole::Member,
        )
        .await
        .expect_err("refused");
        assert!(refused.contains("no such person here"), "{refused}");

        let people = accounts::of_scope(&here.database, here.rival.tenant_id.as_deref())
            .await
            .expect("people");
        let found = people
            .iter()
            .find(|person| person.id == here.rival.id)
            .expect("still here");
        assert_eq!(found.role, NodeRole::Admin);
    }

    #[tokio::test]
    async fn a_member_cannot_promote_anybody() {
        let here = organisation().await;
        assert!(run(
            &here.database,
            &here.member,
            &here.member.id,
            NodeRole::Admin
        )
        .await
        .is_err());
    }
}
