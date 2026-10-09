use wabot::sqlite::SqliteDatabase;

use crate::accounts;
use crate::authz::node_user;
use crate::entities::account::Account;
use crate::values::roles::NodeRole;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    person: &str,
    role: NodeRole,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let theirs = accounts::of_scope(database, None)
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

    /// The page lists `of_scope(None)`, so it never offers one of an
    /// organisation's people. Before this use case existed the POST took
    /// any id at all, which is *hidden but not enforced* — the fault
    /// `/node/capabilities` had.
    #[tokio::test]
    async fn the_operator_does_not_reach_into_an_organisation() {
        let here = organisation().await;
        let refused = run(
            &here.database,
            &here.operator,
            &here.admin.id,
            NodeRole::Member,
        )
        .await
        .expect_err("refused");
        assert!(refused.contains("no such person here"), "{refused}");

        let still = accounts::of_scope(&here.database, here.admin.tenant_id.as_deref())
            .await
            .expect("people");
        assert_eq!(
            still
                .iter()
                .find(|person| person.id == here.admin.id)
                .expect("still here")
                .role,
            NodeRole::Admin
        );
    }

    #[tokio::test]
    async fn an_organisations_administrator_cannot_run_it() {
        let here = organisation().await;
        assert!(run(
            &here.database,
            &here.admin,
            &here.operator.id,
            NodeRole::Member
        )
        .await
        .is_err());
    }
}
