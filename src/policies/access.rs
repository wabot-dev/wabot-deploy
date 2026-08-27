//! Membership, and the one place that answers "may they".
//!
//! Every handler asks this module and nothing else. The alternative —
//! each page comparing roles for itself — is thirty comparisons that
//! have to stay in agreement, and the one that drifts is a page that
//! shows somebody a project they were removed from.

use serde::Serialize;
use wabot::sqlite::rusqlite::OptionalExtension;
use wabot::sqlite::SqliteDatabase;

use crate::accounts::roles::{Access, NodeRole, ProjectRole};
use crate::accounts::Account;

use crate::clock::now_ms;
use crate::entities::project::Project;
use crate::platform::{PlatformError, PlatformResult};

/// Somebody's place in a project, as the members page shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Member {
    pub account_id: String,
    pub username: String,
    pub role: ProjectRole,
}

/// What this account may do about this project.
///
/// The only way to get an [`Access`]. Handlers cannot construct one
/// from a role they read themselves, which is the point: there is one
/// path from "who is asking" to "what may they do".
pub async fn of(
    database: &SqliteDatabase,
    account: &Account,
    project_id: &str,
) -> PlatformResult<Access> {
    if account.role == NodeRole::Admin {
        // **An administrator of a scope, not of the machine.** Without
        // the second half this hands a tenant's administrator every
        // project on the node — its own and every other tenant's — and
        // it does it through the one function every page trusts, which
        // is exactly the blast radius that makes a single door worth
        // having and worth getting right.
        //
        // Asked of the project rather than of a list, because this is
        // the function a page hands one id to; `projects_for` narrows
        // the same way in SQL, for the same reason it always did.
        let scope = crate::repositories::projects::find(database, project_id)
            .await?
            .and_then(|project| project.tenant_id);
        return Ok(match (scope.as_deref(), account.tenant_id.as_deref()) {
            // Their own scope, whole.
            (theirs, mine) if theirs == mine => Access::ADMIN,
            // **The machine's operator may look into a tenant, and only
            // look.** Somebody has to be able to say what this node is
            // running, and `projects_for` lists a tenant's projects for
            // them — a door that showed them the list and refused the
            // page would be two answers to one question.
            //
            // Read and nothing else, because a tenant's project is
            // administered from inside the tenant. It is the same shape a
            // service that arrived on an errand already has: visible,
            // provenanced, and not this node's to edit.
            (Some(_), None) => Access::member(ProjectRole::Viewer),
            // And never the other way. A tenant reaching the node's own
            // projects, or another tenant's, is the frontier failing.
            _ => Access::NONE,
        });
    }

    let (account_id, project_id) = (account.id.clone(), project_id.to_string());
    let role: Option<String> = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT \"role\" FROM membership \
                     WHERE \"account_id\" = ?1 AND \"project_id\" = ?2",
                    (account_id, project_id),
                    |row| row.get(0),
                )
                .optional()
        })
        .await?;

    Ok(match role {
        Some(role) => Access::member(ProjectRole::parse(&role)),
        None => Access::NONE,
    })
}

/// The projects this account may see.
///
/// Not "every project, filtered by the page" — the filter is the
/// query. A list built from everything and narrowed later is one that
/// leaks the moment somebody adds a page that forgets to narrow.
pub async fn projects_for(
    database: &SqliteDatabase,
    account: &Account,
) -> PlatformResult<Vec<Project>> {
    if account.role == NodeRole::Admin {
        // Every project **of their own scope**. For the node's operator
        // that is still everything, tenants included — somebody has to
        // be able to see what the machine is running — and for a
        // tenant's administrator it is theirs and nothing else.
        //
        // Narrowed in the query rather than after it, which is the rule
        // this module was written around: a list built from everything
        // and filtered later leaks the day somebody adds a page that
        // forgets to filter.
        return match &account.tenant_id {
            None => crate::repositories::projects::all(database).await,
            Some(tenant) => of_tenant(database, tenant).await,
        };
    }

    let account_id = account.id.clone();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT p.\"id\", p.\"name\", p.\"slug\", p.\"created_at\", \
                     p.\"origin_node_id\", p.\"tenant_id\" \
                     FROM project p JOIN membership m ON m.\"project_id\" = p.\"id\" \
                     WHERE m.\"account_id\" = ?1 ORDER BY p.\"created_at\" ASC",
                )?
                .query_map([account_id], |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        slug: row.get(2)?,
                        created_at: row.get(3)?,
                        origin_node_id: row.get(4)?,
                        tenant_id: row.get(5)?,
                    })
                })?
                .collect()
        })
        .await?)
}

/// Every project of one tenant.
pub async fn of_tenant(database: &SqliteDatabase, tenant_id: &str) -> PlatformResult<Vec<Project>> {
    let tenant_id = tenant_id.to_string();
    Ok(database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT \"id\", \"name\", \"slug\", \"created_at\", \"origin_node_id\", \
                     \"tenant_id\" FROM project WHERE \"tenant_id\" = ?1 \
                     ORDER BY \"created_at\" ASC",
                )?
                .query_map([tenant_id], |row| {
                    Ok(Project {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        slug: row.get(2)?,
                        created_at: row.get(3)?,
                        origin_node_id: row.get(4)?,
                        tenant_id: row.get(5)?,
                    })
                })?
                .collect()
        })
        .await?)
}

/// The project this account may see under that slug, if any.
///
/// `None` covers both "no such project" and "not yours", deliberately:
/// telling them apart turns the project list into something anybody
/// can enumerate by guessing names.
pub async fn find_project(
    database: &SqliteDatabase,
    account: &Account,
    slug: &str,
) -> PlatformResult<Option<(Project, Access)>> {
    let Some(project) = crate::repositories::projects::find(database, slug).await? else {
        return Ok(None);
    };
    let access = of(database, account, &project.id).await?;
    Ok(access.may_read().then_some((project, access)))
}

/// Put somebody in a project, or change what they are in it.
pub async fn grant(
    database: &SqliteDatabase,
    account_id: &str,
    project_id: &str,
    role: ProjectRole,
) -> PlatformResult<()> {
    let (account_id, project_id) = (account_id.to_string(), project_id.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "INSERT INTO membership (\"account_id\", \"project_id\", \"role\", \"created_at\") \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (\"account_id\", \"project_id\") DO UPDATE SET \
                   \"role\" = excluded.\"role\"",
                (account_id, project_id, role.as_str(), now_ms()),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Take somebody out of a project.
///
/// Refuses to remove the last owner: a project nobody owns is one
/// nobody can grant access to, and the only way back is an
/// administrator noticing.
pub async fn revoke(
    database: &SqliteDatabase,
    account_id: &str,
    project_id: &str,
) -> PlatformResult<()> {
    let owners = members(database, project_id)
        .await?
        .into_iter()
        .filter(|member| member.role == ProjectRole::Owner)
        .collect::<Vec<_>>();

    if owners.len() == 1 && owners[0].account_id == account_id {
        return Err(PlatformError::Refused(
            "this is the project's only owner — make somebody else an owner first".into(),
        ));
    }

    let (account_id, project_id) = (account_id.to_string(), project_id.to_string());
    database
        .write(move |connection| {
            connection.execute(
                "DELETE FROM membership WHERE \"account_id\" = ?1 AND \"project_id\" = ?2",
                (account_id, project_id),
            )?;
            Ok(())
        })
        .await?;
    Ok(())
}

/// Everybody in a project, owners first.
pub async fn members(database: &SqliteDatabase, project_id: &str) -> PlatformResult<Vec<Member>> {
    let project_id = project_id.to_string();
    let rows: Vec<(String, String, String)> = database
        .read(move |connection| {
            connection
                .prepare(
                    "SELECT a.\"id\", a.\"username\", m.\"role\" \
                     FROM membership m JOIN account a ON a.\"id\" = m.\"account_id\" \
                     WHERE m.\"project_id\" = ?1 ORDER BY a.\"username\"",
                )?
                .query_map([project_id], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })?
                .collect()
        })
        .await?;

    let mut members: Vec<Member> = rows
        .into_iter()
        .map(|(account_id, username, role)| Member {
            account_id,
            username,
            role: ProjectRole::parse(&role),
        })
        .collect();

    // Most capable first: who owns this is the question a members list
    // is usually opened to answer.
    members.sort_by(|a, b| b.role.cmp(&a.role).then(a.username.cmp(&b.username)));
    Ok(members)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts;

    async fn node() -> (SqliteDatabase, Account, Account) {
        let database = crate::db::open_in_memory().await.expect("open");
        let token = accounts::issue_setup_token(&database).await.expect("token");
        let admin = accounts::create_admin(&database, &token, "admin", "a long enough passphrase")
            .await
            .expect("admin");
        let member = accounts::create_in(
            &database,
            "member",
            "another long passphrase",
            NodeRole::Member,
            None,
        )
        .await
        .expect("member");
        (database, admin, member)
    }

    /// A tenant's administrator reaches their own projects and nothing
    /// else.
    ///
    /// **The one door, and the biggest thing behind it.** `of` short-
    /// circuits on the role, so without the scope beside it a tenant's
    /// administrator would be handed `Access::ADMIN` for every project on
    /// the machine — the node's own and every other tenant's — through
    /// the function every page in this console trusts to be the only
    /// answer. One clause, and the blast radius of getting it wrong is
    /// the reason a single door is worth having.
    #[tokio::test]
    async fn a_tenants_administrator_reaches_only_that_tenants_projects() {
        let (database, node_admin, _) = node().await;
        let tenant = crate::repositories::tenants::create(&database, "Acme")
            .await
            .expect("tenant");
        let other = crate::repositories::tenants::create(&database, "Globex")
            .await
            .expect("tenant");

        let theirs =
            crate::repositories::projects::create_for(&database, "theirs", Some(&tenant.id))
                .await
                .expect("project");
        let rivals =
            crate::repositories::projects::create_for(&database, "rivals", Some(&other.id))
                .await
                .expect("project");
        let nodes_own = crate::repositories::projects::create(&database, "the nodes own")
            .await
            .expect("project");

        let admin = Account {
            tenant_id: Some(tenant.id.clone()),
            ..node_admin.clone()
        };

        assert!(
            of(&database, &admin, &theirs.id)
                .await
                .expect("access")
                .may_administer(),
            "an administrator of a tenant administers its projects"
        );
        for elsewhere in [&rivals.id, &nodes_own.id] {
            assert_eq!(
                of(&database, &admin, elsewhere).await.expect("access"),
                Access::NONE,
                "a tenant reached past its own frontier"
            );
        }

        // And the list is narrowed in the query, not by the page.
        let seen: Vec<String> = projects_for(&database, &admin)
            .await
            .expect("projects")
            .into_iter()
            .map(|project| project.slug)
            .collect();
        assert_eq!(seen, vec!["theirs".to_string()]);

        // The node's own operator still sees the machine whole —
        // somebody has to be able to say what it is running.
        let all: Vec<String> = projects_for(&database, &node_admin)
            .await
            .expect("projects")
            .into_iter()
            .map(|project| project.slug)
            .collect();
        assert_eq!(all.len(), 3, "{all:?}");
    }

    /// Being the node's administrator and being a tenant's are different
    /// things, and one function tells them apart.
    ///
    /// **Forty-three checks ask `is_admin`**, and the one that forgot a
    /// third case would be a customer holding the machine's own settings
    /// page. So the distinction lives inside the method rather than at
    /// the call sites, which is the same argument `roles.rs` makes about
    /// spelling a comparison out thirty times.
    #[tokio::test]
    async fn administering_a_tenant_is_not_administering_the_machine() {
        let (database, node_admin, _) = node().await;
        let tenant = crate::repositories::tenants::create(&database, "Acme")
            .await
            .expect("tenant");
        let admin = Account {
            tenant_id: Some(tenant.id),
            ..node_admin.clone()
        };

        assert!(node_admin.is_admin() && node_admin.administers_scope());
        assert!(
            !admin.is_admin(),
            "a tenant's administrator reached the node's own pages"
        );
        assert!(
            admin.administers_scope(),
            "and cannot manage their own organisation"
        );
    }

    /// A session belongs to the frontier it was minted in.
    ///
    /// The `Host` decides the scope before anything reads a cookie, so
    /// this is what keeps the two in agreement. Nothing a stray session
    /// reached would be theirs — `of` narrows either way — but a
    /// boundary that holds only because every query remembered to narrow
    /// is not a boundary.
    #[tokio::test]
    async fn a_session_does_not_cross_between_scopes() {
        let (database, node_admin, _) = node().await;
        let tenant = crate::repositories::tenants::create(&database, "Acme")
            .await
            .expect("tenant");
        let theirs = Account {
            tenant_id: Some(tenant.id.clone()),
            ..node_admin.clone()
        };

        assert!(node_admin.belongs_to(None));
        assert!(theirs.belongs_to(Some(&tenant.id)));
        assert!(
            !theirs.belongs_to(None),
            "a tenant's session was accepted on the node's own console"
        );
        assert!(
            !node_admin.belongs_to(Some(&tenant.id)),
            "the operator's session was accepted on a tenant's console"
        );
        assert!(
            !theirs.belongs_to(Some("tnt-somebodyelse")),
            "a session crossed between tenants"
        );
    }

    #[tokio::test]
    async fn an_administrator_reaches_every_project_without_membership() {
        let (database, admin, _) = node().await;
        let project = crate::repositories::projects::create(&database, "theirs")
            .await
            .expect("project");

        let access = of(&database, &admin, &project.id).await.expect("access");
        assert!(access.may_administer());
        assert_eq!(
            projects_for(&database, &admin).await.expect("list").len(),
            1
        );
    }

    /// The property the whole model rests on: a project you are not in
    /// is a project you cannot see.
    #[tokio::test]
    async fn a_member_sees_only_the_projects_they_are_in() {
        let (database, _, member) = node().await;
        let theirs = crate::repositories::projects::create(&database, "theirs")
            .await
            .expect("project");
        let other = crate::repositories::projects::create(&database, "somebody elses")
            .await
            .expect("project");

        grant(&database, &member.id, &theirs.id, ProjectRole::Deployer)
            .await
            .expect("grant");

        let visible = projects_for(&database, &member).await.expect("list");
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, theirs.id);

        assert!(!of(&database, &member, &other.id)
            .await
            .expect("access")
            .may_read());
    }

    /// "Not yours" and "does not exist" have to be the same answer, or
    /// the names of every project on the node are discoverable by
    /// asking for them one at a time.
    #[tokio::test]
    async fn a_project_you_cannot_see_is_indistinguishable_from_one_that_is_not_there() {
        let (database, _, member) = node().await;
        crate::repositories::projects::create(&database, "secret")
            .await
            .expect("project");

        assert_eq!(
            find_project(&database, &member, "secret")
                .await
                .expect("find"),
            None
        );
        assert_eq!(
            find_project(&database, &member, "never-existed")
                .await
                .expect("find"),
            None
        );
    }

    #[tokio::test]
    async fn a_role_can_be_changed_without_a_second_row() {
        let (database, _, member) = node().await;
        let project = crate::repositories::projects::create(&database, "p")
            .await
            .expect("project");

        grant(&database, &member.id, &project.id, ProjectRole::Viewer)
            .await
            .expect("grant");
        grant(&database, &member.id, &project.id, ProjectRole::Owner)
            .await
            .expect("regrant");

        let members = members(&database, &project.id).await.expect("members");
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].role, ProjectRole::Owner);
    }

    /// A project with no owner is one nobody can grant access to, and
    /// the only way back is an administrator noticing.
    #[tokio::test]
    async fn the_last_owner_cannot_be_removed() {
        let (database, _, member) = node().await;
        let project = crate::repositories::projects::create(&database, "p")
            .await
            .expect("project");
        grant(&database, &member.id, &project.id, ProjectRole::Owner)
            .await
            .expect("grant");

        let error = revoke(&database, &member.id, &project.id)
            .await
            .expect_err("refused");
        assert!(error.to_string().contains("only owner"), "{error}");
    }

    #[tokio::test]
    async fn an_owner_can_leave_once_there_is_another() {
        let (database, admin, member) = node().await;
        let project = crate::repositories::projects::create(&database, "p")
            .await
            .expect("project");
        grant(&database, &member.id, &project.id, ProjectRole::Owner)
            .await
            .expect("grant");
        grant(&database, &admin.id, &project.id, ProjectRole::Owner)
            .await
            .expect("grant");

        revoke(&database, &member.id, &project.id)
            .await
            .expect("revoked");
        assert_eq!(
            members(&database, &project.id)
                .await
                .expect("members")
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn members_are_listed_with_the_owners_first() {
        let (database, admin, member) = node().await;
        let project = crate::repositories::projects::create(&database, "p")
            .await
            .expect("project");
        grant(&database, &member.id, &project.id, ProjectRole::Viewer)
            .await
            .expect("grant");
        grant(&database, &admin.id, &project.id, ProjectRole::Owner)
            .await
            .expect("grant");

        let members = members(&database, &project.id).await.expect("members");
        assert_eq!(members[0].role, ProjectRole::Owner);
        assert_eq!(members[1].role, ProjectRole::Viewer);
    }

    /// Deleting a project takes its memberships with it, or the next
    /// project to reuse an id inherits them.
    #[tokio::test]
    async fn memberships_go_with_the_project() {
        let (database, _, member) = node().await;
        let project = crate::repositories::projects::create(&database, "p")
            .await
            .expect("project");
        grant(&database, &member.id, &project.id, ProjectRole::Owner)
            .await
            .expect("grant");

        crate::repositories::projects::delete(&database, &project.id)
            .await
            .expect("delete");
        assert!(members(&database, &project.id)
            .await
            .expect("members")
            .is_empty());
    }
}
