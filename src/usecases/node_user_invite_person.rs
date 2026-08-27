use wabot::sqlite::SqliteDatabase;

use crate::accounts::invitations;
use crate::accounts::roles::{NodeRole, ProjectRole};
use crate::accounts::Account;
use crate::authz::node_user;
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
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can invite somebody".into());
    }

    let project = match asked.project.filter(|slug| !slug.trim().is_empty()) {
        None => None,
        Some(slug) => match access::find_project(database, account, slug)
            .await
            .map_err(|error| error.to_string())?
        {
            // An invitation minted here is into the machine, so one
            // carrying membership of an organisation's project would
            // make somebody who reaches exactly one project and no
            // console that admits they exist.
            Some((project, _)) if project.is_the_nodes_own() => {
                Some((project.id, asked.project_role))
            }
            Some(_) => {
                return Err(
                    "that project belongs to an organisation — invite them from its own page"
                        .into(),
                )
            }
            None => return Err("no such project".into()),
        },
    };

    let token = invitations::create_into(
        database,
        account,
        asked.role,
        project.as_ref().map(|(id, role)| (id.as_str(), *role)),
        None,
    )
    .await
    .map_err(|error| error.to_string())?;

    Ok(Invited { token })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecases::fixtures::organisation;

    fn asked() -> Asked<'static> {
        Asked {
            role: NodeRole::Member,
            project: None,
            project_role: ProjectRole::Viewer,
        }
    }

    #[tokio::test]
    async fn the_link_opens_the_machine_itself() {
        let here = organisation().await;
        let invited = run(&here.database, &here.operator, asked())
            .await
            .expect("invited");

        let found = invitations::look_up(&here.database, &invited.token)
            .await
            .expect("query")
            .expect("live");
        assert_eq!(found.tenant_id, None);
    }

    #[tokio::test]
    async fn an_organisations_administrator_cannot_run_it() {
        let here = organisation().await;
        let refused = run(&here.database, &here.admin, asked())
            .await
            .expect_err("refused");
        assert!(refused.contains("administrator of this node"), "{refused}");
    }
}
