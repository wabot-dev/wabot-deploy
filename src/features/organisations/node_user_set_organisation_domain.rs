use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::infra::deploy::dns::Resolution;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::tenants;

pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    organisation: &str,
    wanted: &str,
) -> Result<(), String> {
    if !node_user::shares_the_machine_out(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let wanted = wanted.trim().to_lowercase();
    let Some(tenant) = tenants::find(database, organisation)
        .await
        .map_err(|error| error.to_string())?
    else {
        return Err("no such organisation".into());
    };

    if !wanted.is_empty() {
        // Refused here as well as in `routing::sync`: the sync skipping
        // it would leave a row saved on a page that shows it and a
        // console that never answers there, which is a setting that lies.
        if crate::infra::node::settings::domain(database)
            .await
            .as_deref()
            == Some(wanted.as_str())
        {
            return Err(
                "that is this node's own name — the console an operator fixes things \
                        from cannot be somebody else's frontier"
                    .into(),
            );
        }
        // Two rows claiming one `Host` is not something a request can
        // resolve, and choosing silently would make the wrong one look
        // right.
        if let Ok(Some(held)) = tenants::by_domain(database, &wanted).await {
            if held.id != tenant.id {
                return Err(crate::shell::texts::sentences::name_is_taken(&held.name));
            }
        }
        // This node is the organisation's console, so its name has to
        // arrive here — the question the node's own domain and every
        // service hostname are asked before they are saved. Not asked, a
        // typo was saved, routed and shown, and the console never
        // answered on it. Found on a VM node with `nowhere.invalid`.
        let node_domain = crate::infra::node::settings::domain(database).await;
        let reference = node_domain.as_deref().unwrap_or(&wanted);
        let outcome = match crate::infra::deploy::dns::resolves_here(&wanted, reference).await {
            Resolution::NoReference if node_domain.is_none() => Resolution::Missing,
            outcome => outcome,
        };
        if !outcome.ok() {
            return Err(outcome.explain(&wanted));
        }
    }

    let domain = (!wanted.is_empty()).then_some(wanted);
    tenants::set_domain(database, &tenant.id, domain.as_deref())
        .await
        .map_err(|error| error.to_string())?;

    // The route follows the row, here rather than at the next
    // deployment: a domain saved and not routed is a console that
    // answers 404 on the name the page says it is at.
    deployer.sync_routes().await;
    Ok(())
}
