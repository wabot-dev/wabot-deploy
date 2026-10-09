use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::accounts::invitations;

/// The machine's operator withdraws any invitation the node holds, an
/// organisation's included: they mint its first one from the
/// organisations page, and an invitation nobody can take back is a door
/// left open.
pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    invitation: &str,
) -> Result<(), String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let known = invitations::all(database)
        .await
        .map_err(|error| error.to_string())?;
    if !known.iter().any(|found| found.id == invitation) {
        return Err("no such invitation".into());
    }

    invitations::revoke(database, invitation)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::roles::{NodeRole, ProjectRole};
    use crate::features::fixtures::organisation;
    use crate::features::people::invite_person_request::InvitePersonRequest;
    use crate::features::people::tenant_user_invite_person;

    #[tokio::test]
    async fn the_operator_withdraws_an_organisations_invitation() {
        let here = organisation().await;
        tenant_user_invite_person::run(
            &here.database,
            &here.admin,
            &InvitePersonRequest {
                role: NodeRole::Member,
                project: String::new(),
                project_role: ProjectRole::Viewer,
            },
        )
        .await
        .expect("invited");

        let theirs = invitations::all(&here.database)
            .await
            .expect("invitations")
            .into_iter()
            .find(|found| found.tenant_id == here.admin.tenant_id)
            .expect("theirs");
        assert_eq!(theirs.scope_name.as_deref(), Some("Acme"));

        run(&here.database, &here.operator, &theirs.id)
            .await
            .expect("withdrawn");
        assert!(invitations::all(&here.database)
            .await
            .expect("invitations")
            .iter()
            .all(|found| found.id != theirs.id));
    }

    #[tokio::test]
    async fn an_organisations_administrator_cannot_run_it() {
        let here = organisation().await;
        assert!(run(&here.database, &here.admin, "inv-whatever")
            .await
            .is_err());
    }
}
