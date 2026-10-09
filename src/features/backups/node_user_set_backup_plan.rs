use wabot::sqlite::SqliteDatabase;

use crate::authz::node_user;
use crate::backup::destination::Destination;
use crate::backup::plan::{self, Cadence, Plan};
use crate::config::Config;
use crate::entities::account::Account;
use crate::features::backups::backup_plan_saved_response::BackupPlanSavedResponse;
use crate::features::backups::set_backup_plan_request::SetBackupPlanRequest;

pub async fn run(
    database: &SqliteDatabase,
    config: &Config,
    account: &Account,
    asked: &SetBackupPlanRequest,
) -> Result<BackupPlanSavedResponse, String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    let typed = asked.destination.trim().to_string();
    let destination = match typed.is_empty() {
        true => None,
        false => {
            let destination = Destination::parse(&typed)?;
            destination.preflight()?;
            if matches!(destination, Destination::S3 { .. }) {
                let Some(credentials) = config.s3() else {
                    return Err(
                        "that bucket has no credentials on this node — the card below \
                                stores them, in the config file rather than the database"
                            .into(),
                    );
                };
                if let Some(reason) = bucket_refusal(Some(&typed), &credentials).await {
                    return Err(reason);
                }
            }
            Some(typed)
        }
    };

    let keep_days = match asked.keep_days.trim() {
        "" => crate::backup::KEEP_DAYS,
        text => match text.parse::<i64>() {
            Ok(days) if days >= plan::MIN_KEEP_DAYS => days,
            Ok(_) => {
                return Err(
                    "a window of no days would delete each backup as soon as it was taken".into(),
                )
            }
            Err(_) => return Err(format!("{text:?} is not a number of days")),
        },
    };

    let plan = Plan {
        destination,
        cadence: Cadence::parse(asked.cadence.trim()),
        hour: asked
            .hour
            .trim()
            .parse()
            .unwrap_or(Plan::default().hour)
            .min(23),
        weekday: asked
            .weekday
            .trim()
            .parse()
            .unwrap_or(Plan::default().weekday)
            .min(plan::SUNDAY),
        keep_days,
    };
    plan::store(database, &plan).await.map_err(|error| {
        tracing::error!(%error, "could not store the backup plan");
        "that could not be saved".to_string()
    })?;

    let was = crate::node::settings::archiving(database).await;
    let wanted = asked.archiving;
    let archiving_changed = match crate::node::settings::set_archiving(database, wanted).await {
        Ok(()) => was != wanted,
        Err(error) => {
            tracing::warn!(%error, "could not save whether to keep the log");
            false
        }
    };

    Ok(BackupPlanSavedResponse { archiving_changed })
}

/// Ask the bucket, and say what it said — or `None` if there is nothing
/// to ask.
///
/// **A listing.** It needs the same signature, the same host and the same
/// permission a backup needs, and `send_to_s3` opens with one anyway — so
/// this is not a weaker check than the real thing, and it writes nothing
/// into somebody's bucket to find out.
///
/// Bounded, because a form that waits on a provider is a page that hangs:
/// a wrong endpoint is a connection nobody answers, and fifteen seconds is
/// long enough to be a slow bucket and short enough to be an answer.
pub(crate) async fn bucket_refusal(
    destination: Option<&str>,
    credentials: &crate::backup::s3::Credentials,
) -> Option<String> {
    let Some(crate::backup::destination::Destination::S3 { bucket, prefix }) =
        destination.and_then(|text| crate::backup::destination::Destination::parse(text).ok())
    else {
        return None;
    };
    match tokio::time::timeout(
        std::time::Duration::from_secs(15),
        crate::backup::s3::list(credentials, &bucket, &prefix),
    )
    .await
    {
        Ok(Ok(_)) => None,
        // The provider's own words. Never translated and never
        // paraphrased: "SignatureDoesNotMatch" is what an operator will
        // search for.
        Ok(Err(error)) => Some(format!("{bucket} refused it: {error}")),
        Err(_) => Some(format!(
            "{bucket} did not answer within fifteen seconds — check the endpoint"
        )),
    }
}
