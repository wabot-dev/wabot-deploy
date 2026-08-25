use wabot::sqlite::SqliteDatabase;

use crate::accounts::{self, Account};
use crate::authz::node_user;

pub async fn run(database: &SqliteDatabase, account: &Account, person: &str) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if person == account.id {
        return Err("removing yourself would leave you signed in to nothing".into());
    }

    let theirs = accounts::of_scope(database, None)
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
    use crate::usecases::fixtures::organisation;

    #[tokio::test]
    async fn the_operator_does_not_reach_into_an_organisation() {
        let here = organisation().await;
        let refused = run(&here.database, &here.operator, &here.member.id)
            .await
            .expect_err("refused");
        assert!(refused.contains("no such person here"), "{refused}");

        assert!(
            accounts::of_scope(&here.database, here.member.tenant_id.as_deref())
                .await
                .expect("people")
                .iter()
                .any(|person| person.id == here.member.id)
        );
    }

    #[tokio::test]
    async fn nobody_removes_themselves() {
        let here = organisation().await;
        let refused = run(&here.database, &here.operator, &here.operator.id)
            .await
            .expect_err("refused");
        assert!(refused.contains("signed in to nothing"), "{refused}");
    }
}
