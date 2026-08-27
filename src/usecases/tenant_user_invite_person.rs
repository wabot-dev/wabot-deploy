use wabot::sqlite::SqliteDatabase;

use crate::accounts::invitations;
use crate::accounts::roles::{NodeRole, ProjectRole};
use crate::accounts::Account;
use crate::authz::tenant_user;
use crate::policies::access;

pub struct Asked<'a> {
    pub role: NodeRole,
    pub project: Option<&'a str>,
    pub project_role: ProjectRole,
}

#[derive(Debug)]
pub struct Invited {
    pub token: String,
}

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: Asked<'_>,
) -> Result<Invited, String> {
    if !tenant_user::administers(account) {
        return Err("only an administrator of this organisation can invite somebody".into());
    }

    let project = match asked.project.filter(|slug| !slug.trim().is_empty()) {
        None => None,
        Some(slug) => match access::find_project(database, account, slug)
            .await
            .map_err(|error| error.to_string())?
        {
            Some((project, _)) => Some((project.id, asked.project_role)),
            None => return Err("no such project".into()),
        },
    };

    let token = invitations::create_into(
        database,
        account,
        asked.role,
        project.as_ref().map(|(id, role)| (id.as_str(), *role)),
        account.tenant_id.as_deref(),
    )
    .await
    .map_err(|error| error.to_string())?;

    Ok(Invited { token })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts;
    use crate::usecases::fixtures::organisation;

    fn asked() -> Asked<'static> {
        Asked {
            role: NodeRole::Member,
            project: None,
            project_role: ProjectRole::Viewer,
        }
    }

    #[tokio::test]
    async fn the_link_opens_the_organisation_it_was_made_in() {
        let here = organisation().await;
        let invited = run(&here.database, &here.admin, asked())
            .await
            .expect("invited");

        let found = accounts::invitations::look_up(&here.database, &invited.token)
            .await
            .expect("query")
            .expect("live");
        assert_eq!(found.tenant_id, here.admin.tenant_id);
    }

    /// The machine's operator is not one of this organisation's people,
    /// so this use case is not theirs to run — they have their own.
    #[tokio::test]
    async fn the_machines_operator_cannot_run_it() {
        let here = organisation().await;
        let refused = run(&here.database, &here.operator, asked())
            .await
            .expect_err("refused");
        assert!(
            refused.contains("administrator of this organisation"),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn a_member_cannot_invite() {
        let here = organisation().await;
        assert!(run(&here.database, &here.member, asked()).await.is_err());
    }

    #[tokio::test]
    async fn a_project_outside_the_organisation_is_not_found() {
        let here = organisation().await;
        let elsewhere = crate::repositories::projects::create(&here.database, "the nodes own")
            .await
            .expect("project");

        let refused = run(
            &here.database,
            &here.admin,
            Asked {
                project: Some(&elsewhere.slug),
                ..asked()
            },
        )
        .await
        .expect_err("refused");
        assert!(refused.contains("no such project"), "{refused}");
    }
}
