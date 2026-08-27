use wabot::sqlite::SqliteDatabase;

use crate::accounts::invitations;
use crate::accounts::roles::{NodeRole, ProjectRole};
use crate::accounts::Account;
use crate::authz::node_user;
use crate::platform::{access, tenants};

pub struct Asked<'a> {
    pub organisation: &'a str,
    pub role: NodeRole,
    pub project: Option<&'a str>,
    pub project_role: ProjectRole,
}

#[derive(Debug)]
pub struct Invited {
    pub organisation: String,
    pub token: String,
}

/// The only way into an organisation, since one with nobody in it cannot
/// invite anybody either.
pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    asked: Asked<'_>,
) -> Result<Invited, String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let Some(tenant) = tenants::find(database, asked.organisation)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such organisation".into());
    };

    let project = match asked.project.filter(|slug| !slug.trim().is_empty()) {
        None => None,
        Some(slug) => match access::find_project(database, account, slug)
            .await
            .map_err(|error| error.to_string())?
        {
            // An operator sees every project the machine holds, so
            // without this a link into one organisation could carry
            // membership of another's — somebody who reaches exactly one
            // project and no console that admits they exist.
            Some((project, _)) if project.tenant_id.as_deref() == Some(&*tenant.id) => {
                Some((project.id, asked.project_role))
            }
            Some(_) => return Err("that project is not in this organisation".into()),
            None => return Err("no such project".into()),
        },
    };

    let token = invitations::create_into(
        database,
        account,
        asked.role,
        project.as_ref().map(|(id, role)| (id.as_str(), *role)),
        Some(&tenant.id),
    )
    .await
    .map_err(|error| error.to_string())?;

    Ok(Invited {
        organisation: tenant.id,
        token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::usecases::fixtures::organisation;

    fn asked(organisation: &str) -> Asked<'_> {
        Asked {
            organisation,
            role: NodeRole::Member,
            project: None,
            project_role: ProjectRole::Viewer,
        }
    }

    #[tokio::test]
    async fn the_link_lands_in_the_organisation_it_names() {
        let here = organisation().await;
        let acme = here.admin.tenant_id.clone().expect("scoped");

        let invited = run(&here.database, &here.operator, asked(&acme))
            .await
            .expect("invited");
        let found = invitations::look_up(&here.database, &invited.token)
            .await
            .expect("query")
            .expect("live");
        assert_eq!(found.tenant_id.as_deref(), Some(acme.as_str()));
    }

    /// The machine's own projects are visible to the operator, so
    /// nothing but this stops a link into one organisation carrying
    /// membership of a project outside it.
    #[tokio::test]
    async fn a_project_outside_the_organisation_is_refused() {
        let here = organisation().await;
        let acme = here.admin.tenant_id.clone().expect("scoped");
        let elsewhere = crate::repositories::projects::create(&here.database, "the nodes own")
            .await
            .expect("project");

        let refused = run(
            &here.database,
            &here.operator,
            Asked {
                project: Some(&elsewhere.slug),
                ..asked(&acme)
            },
        )
        .await
        .expect_err("refused");
        assert!(refused.contains("not in this organisation"), "{refused}");
    }

    #[tokio::test]
    async fn an_organisations_administrator_cannot_run_it() {
        let here = organisation().await;
        let acme = here.admin.tenant_id.clone().expect("scoped");
        assert!(run(&here.database, &here.admin, asked(&acme))
            .await
            .is_err());
    }
}
