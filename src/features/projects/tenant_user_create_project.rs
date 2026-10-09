use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::domain::entities::project::Project;
use crate::features::projects::create_project_command::CreateProject;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    name: &str,
) -> Result<Project, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to make".into());
    }

    CreateProject { name, by: account }.run(database).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::fixtures::organisation;

    /// A project made inside an organisation is the organisation's.
    ///
    /// **Shipped otherwise**: every project was made as the node's own, so
    /// one an organisation's owner created escaped its quota and its count,
    /// and survived its deletion. Found on a VM node.
    #[tokio::test]
    async fn a_project_made_inside_an_organisation_belongs_to_it() {
        let here = organisation().await;
        let made = run(&here.database, &here.admin, "storefront")
            .await
            .expect("made");
        assert!(made.tenant_id.is_some());
        assert_eq!(made.tenant_id, here.admin.tenant_id);
    }
}
