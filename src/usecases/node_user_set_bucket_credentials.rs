use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::s3::Credentials;
use crate::config::Config;
use crate::usecases::node_user_set_backup_plan::bucket_refusal;

pub struct Asked<'a> {
    pub access_key_id: &'a str,
    /// Empty keeps what is stored: a page that showed the secret back to
    /// be re-typed would be a page that shows the secret.
    pub secret_access_key: &'a str,
    pub region: &'a str,
    pub endpoint: &'a str,
}

/// The credential lands in the **config file, never the database** — a
/// database is what a backup copies, and a backup carrying the key to
/// its own bucket is a key in every copy.
pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    asked: Asked<'_>,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let Some(path) = config.source.clone() else {
        return Err(
            "this node was started without a configuration file, so there is nowhere to \
                    keep a credential that must not go in the database"
                .into(),
        );
    };

    let stored = config.s3();
    let secret = match asked.secret_access_key {
        "" => match stored.as_ref().map(|held| held.secret_access_key.clone()) {
            Some(held) => held,
            None => {
                return Err("the secret access key is needed: there is none stored to keep".into())
            }
        },
        typed => typed.to_string(),
    };
    if asked.access_key_id.is_empty() {
        return Err("the access key id is needed".into());
    }
    if asked.region.is_empty() {
        return Err(
            "a region is needed — providers that have no regions still document one, \
                    often us-east-1"
                .into(),
        );
    }

    let credentials = Credentials {
        access_key_id: asked.access_key_id.to_string(),
        secret_access_key: secret,
        region: asked.region.to_string(),
        endpoint: (!asked.endpoint.is_empty()).then(|| asked.endpoint.to_string()),
    };

    let plan = crate::node::backups::plan(database).await;
    if let Some(reason) = bucket_refusal(plan.destination.as_deref(), &credentials).await {
        return Err(reason);
    }

    crate::config::store_s3(&path, Some(&credentials)).map_err(|error| {
        tracing::error!(%error, "could not store the bucket credential");
        error.to_string()
    })
}
