use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::entities::account::Account;
use crate::policies::presets;
use crate::repositories::tenants;

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    organisation: &str,
    memory: &str,
    cpu: &str,
) -> Result<(), String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    // Empty is no ceiling, which is a real answer and not a missing
    // field: an organisation whose quota is lifted has to be able to say
    // so.
    let memory = presets::parse_size(memory)?;
    let cpu = presets::parse_cores(cpu)?;

    tenants::set_quota(database, organisation, memory, cpu, None)
        .await
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::fixtures::organisation;

    #[tokio::test]
    async fn an_empty_ceiling_lifts_the_quota() {
        let here = organisation().await;
        let acme = here.admin.tenant_id.as_deref().expect("scoped");

        run(&here.database, &here.operator, acme, "512 MB", "1")
            .await
            .expect("set");
        run(&here.database, &here.operator, acme, "", "")
            .await
            .expect("lifted");

        let found = tenants::find(&here.database, acme)
            .await
            .expect("query")
            .expect("there");
        assert_eq!(found.quota_memory, None);
        assert_eq!(found.quota_cpu, None);
    }

    #[tokio::test]
    async fn an_organisations_administrator_cannot_raise_their_own_ceiling() {
        let here = organisation().await;
        let acme = here.admin.tenant_id.as_deref().expect("scoped");
        assert!(run(&here.database, &here.admin, acme, "64 GB", "32")
            .await
            .is_err());
    }
}
