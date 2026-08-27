use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::entities::project::Project;
use crate::platform::PlatformResult;
use crate::policies::access;

pub fn belongs_here(account: &Account) -> bool {
    account.tenant_id.is_some()
}

pub fn administers(account: &Account) -> bool {
    belongs_here(account) && account.administers_scope()
}

/// Unused until the service use cases land, and written with the module
/// so a tenant's authority reads as one file.
#[allow(dead_code)]
pub async fn may_deploy(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
) -> PlatformResult<bool> {
    if !is_theirs(account, project) {
        return Ok(false);
    }
    Ok(access::of(database, account, &project.id)
        .await?
        .may_deploy())
}

#[allow(dead_code)]
pub async fn may_administer_project(
    database: &SqliteDatabase,
    account: &Account,
    project: &Project,
) -> PlatformResult<bool> {
    if !is_theirs(account, project) {
        return Ok(false);
    }
    Ok(access::of(database, account, &project.id)
        .await?
        .may_administer())
}

#[allow(dead_code)]
fn is_theirs(account: &Account, project: &Project) -> bool {
    belongs_here(account) && project.tenant_id == account.tenant_id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::roles::NodeRole;

    fn account(tenant: Option<&str>, role: NodeRole) -> Account {
        Account {
            id: "acc-1".into(),
            username: "jorge".into(),
            role,
            theme: crate::console::shell::Theme::System,
            language: crate::console::language::Language::En,
            tenant_id: tenant.map(str::to_string),
        }
    }

    #[test]
    fn the_machines_operator_is_not_one_of_them() {
        assert!(!belongs_here(&account(None, NodeRole::Admin)));
        assert!(!administers(&account(None, NodeRole::Admin)));

        assert!(belongs_here(&account(Some("tnt-acme"), NodeRole::Member)));
        assert!(administers(&account(Some("tnt-acme"), NodeRole::Admin)));
        assert!(!administers(&account(Some("tnt-acme"), NodeRole::Member)));
    }
}
