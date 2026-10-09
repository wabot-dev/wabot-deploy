use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::config::Config;
use crate::features::updates::release_started_response::ReleaseStartedResponse;
use crate::update::runs;
use crate::update::Catalogue;

/// Everything that can be refused is refused **here**, while there is
/// still a request to answer. What happens after this returns is
/// reported on the run row, because the browser that started it is about
/// to lose the server it asked.
pub async fn run(
    database: &Arc<SqliteDatabase>,
    config: &Config,
    catalogue: &Arc<Catalogue>,
    account: &Account,
    tag: &str,
) -> Result<ReleaseStartedResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if tag.is_empty() {
        return Err("no release chosen".into());
    }

    let releases = catalogue
        .releases()
        .await
        .map_err(|error| error.to_string())?;
    let Some(release) = crate::update::github::find(&releases, tag) else {
        return Err(format!("no release is tagged {tag}"));
    };
    if !release.installable() {
        return Err(format!(
            "{} published no build this node can install",
            release.tag
        ));
    }
    if crate::update::github::Version::current() == Some(release.version) {
        return Err(crate::views::texts::sentences::already_running(
            &release.tag,
        ));
    }
    if let Some(existing) = runs::in_flight(database)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err(format!(
            "an update to {} is already running",
            existing.to_version
        ));
    }
    if !crate::bootstrap::service::supervised() {
        return Err("nothing supervises this node, so it cannot restart itself".into());
    }

    crate::update::start_in_background(
        database.clone(),
        config.clone(),
        catalogue.clone(),
        release.tag.clone(),
        Some(account.id.clone()),
    );

    Ok(ReleaseStartedResponse {
        tag: release.tag.clone(),
    })
}
