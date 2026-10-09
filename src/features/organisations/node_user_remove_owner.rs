use wabot::sqlite::SqliteDatabase;

use crate::accounts::roles::NodeRole;
use crate::accounts::{self, Account};
use crate::authz::node_user;

/// Take an organisation's administrator off the machine.
///
/// **This is a hole in the frontier, and it is deliberate.** Everywhere
/// else the node's operator cannot reach inside an organisation — see
/// `node_user_remove_person`, which refuses with *no such person here*.
/// The exception is narrow and it earns itself: an organisation whose
/// only administrator has gone is one nobody can invite anybody into,
/// including a replacement, and the operator would have no way to hand
/// it over short of deleting it with its projects inside.
///
/// So the reach is exactly the people the operator could have created:
/// **administrators, and only them.** A member of an organisation is
/// still theirs to remove, from their own console.
pub async fn run(database: &SqliteDatabase, account: &Account, person: &str) -> Result<(), String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let everybody = accounts::all(database)
        .await
        .map_err(|error| error.to_string())?;
    let Some(found) = everybody.iter().find(|one| one.id == person) else {
        return Err("no such person here".into());
    };
    // The machine's own people are `node_user_remove_person`'s: this one
    // reaches into a frontier, and a use case that did both would be one
    // whose refusals nobody could predict.
    if found.tenant_id.is_none() {
        return Err("that is one of this node's own people".into());
    }
    if found.role != NodeRole::Admin {
        return Err(
            "only an organisation's administrators are the operator's to remove — a member \
             is theirs, from their own console"
                .into(),
        );
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
    async fn the_operator_hands_an_organisation_to_somebody_else() {
        let here = organisation().await;
        run(&here.database, &here.operator, &here.admin.id)
            .await
            .expect("removed");

        assert!(!accounts::all(&here.database)
            .await
            .expect("query")
            .iter()
            .any(|one| one.id == here.admin.id));
    }

    /// A member is the organisation's to remove, not the machine's. The
    /// operator has no business deciding who works inside a frontier
    /// they only host.
    #[tokio::test]
    async fn a_member_of_an_organisation_is_not_theirs_to_remove() {
        let here = organisation().await;
        let refused = run(&here.database, &here.operator, &here.member.id)
            .await
            .expect_err("refused");
        assert!(refused.contains("is theirs"), "{refused}");

        assert!(accounts::all(&here.database)
            .await
            .expect("query")
            .iter()
            .any(|one| one.id == here.member.id));
    }

    /// And the machine's own people go through the use case that says so
    /// — including the operator, who would otherwise reach themselves
    /// through the organisations page.
    #[tokio::test]
    async fn one_of_the_nodes_own_people_goes_the_other_way() {
        let here = organisation().await;
        let refused = run(&here.database, &here.operator, &here.operator.id)
            .await
            .expect_err("refused");
        assert!(refused.contains("this node's own people"), "{refused}");
    }

    #[tokio::test]
    async fn an_organisations_administrator_cannot_run_it() {
        let here = organisation().await;
        assert!(run(&here.database, &here.admin, &here.member.id)
            .await
            .is_err());
    }
}
