use wabot::sqlite::SqliteDatabase;

use crate::accounts::Account;
use crate::authz::node_user;
use crate::commands::destination::Destination;
use crate::config::Config;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    let Some(path) = config.source.clone() else {
        return Err("this node was started without a configuration file".into());
    };

    // Refused while the backups go to a bucket. The button is disabled
    // for the same reason, and this is the check that counts.
    let plan = crate::node::backups::plan(database).await;
    if plan
        .destination
        .as_deref()
        .and_then(|text| Destination::parse(text).ok())
        .is_some_and(|destination| matches!(destination, Destination::S3 { .. }))
    {
        return Err(
            "the backups go to a bucket — change the destination first, or this node \
                    would keep a schedule it cannot carry out"
                .into(),
        );
    }

    crate::config::store_s3(&path, None).map_err(|error| {
        tracing::error!(%error, "could not remove the bucket credential");
        error.to_string()
    })
}
