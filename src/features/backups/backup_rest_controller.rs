//! What the Backup tab of this machine's settings posts to.

use std::sync::Arc;

use wabot::prelude::*;
use wabot::rest::axum::extract::Request;
use wabot::rest::axum::response::Response;
use wabot::rest::RestResult;

use crate::console::auth::{signed_in, SessionMiddleware};
use crate::console::ConsoleState;
use crate::features::node::node_user_node_settings_page::NodeTab;

#[injectable]
pub struct BackupRestController {
    state: Arc<ConsoleState>,
    auth: Arc<Auth>,
}

#[rest_controller("/")]
impl BackupRestController {
    /// Redeploy every managed engine here, because something they read
    /// at startup has changed.
    ///
    /// Queued rather than done: a deployment is this node's own job,
    /// which is what "obeying is local" has meant since phase 3, and a
    /// form that waited for three databases to restart is a form that
    /// times out.
    async fn redeploy_databases(&self) {
        let Ok(services) = crate::repositories::services::all(&self.state.database, None).await
        else {
            return;
        };
        for service in services.iter().filter(|service| service.kind.is_managed()) {
            let command = crate::deploy::jobs::DeployService {
                service_id: service.id.clone(),
                release_id: None,
            };
            if let Err(error) =
                wabot::async_jobs::run_command(&self.state.container, &command).await
            {
                tracing::error!(service = %service.slug, %error, "could not queue it");
            }
        }
    }

    /// What this node backs itself up to, and when.
    ///
    /// **The destination is checked here and not only when the run comes
    /// round.** A schedule saved against `sftp://…` — one letter from a
    /// scheme this understands — is a job that fails at three in the
    /// morning for a reason that was available the moment somebody typed
    /// it. Same rule as the certificate form, which reads a pair of files
    /// before it accepts them.
    #[post("/node/backups")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_backups(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::console::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::console::auth::see_other("/"));
        }
        let form = match crate::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };

        let asked = crate::features::backups::set_backup_plan_request::SetBackupPlanRequest {
            destination: crate::console::auth::field(&form, "destination").to_string(),
            cadence: crate::console::auth::field(&form, "cadence").to_string(),
            hour: crate::console::auth::field(&form, "hour").to_string(),
            weekday: crate::console::auth::field(&form, "weekday").to_string(),
            keep_days: crate::console::auth::field(&form, "keep_days").to_string(),
            // Presence, not value: an unticked checkbox sends nothing at
            // all, so this is the last place that can tell it from a
            // form that never carried the field.
            archiving: form.contains_key("archiving"),
        };

        let saved = crate::features::backups::node_user_set_backup_plan::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await;

        match saved {
            Ok(saved) => {
                // Something the managed engines read at startup moved, so
                // they have to come back with it.
                if saved.archiving_changed {
                    self.redeploy_databases().await;
                }
                Ok(crate::console::auth::see_other(here))
            }
            Err(reason) => Ok(crate::console::auth::back_with_error(here, &reason)),
        }
    }

    /// Store the bucket's credential — in the file, never in the
    /// database.
    ///
    /// Jorge asked for the mechanism: a bucket used to need an SSH
    /// session and a text editor. What did not move is *where* it goes,
    /// and the reasoning is on `config::BackupConfig`: a key that can
    /// read every backup in the network must not be inside the thing
    /// being backed up.
    ///
    /// **Asked of the provider before it is written**, when the
    /// destination is a bucket. The call is a listing, which is what a
    /// backup starts with anyway — so a credential that saves is one that
    /// works, and a typo is refused here rather than at three in the
    /// morning by a schedule nobody is watching.
    #[post("/node/backups/credentials")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn set_bucket_credentials(&self, request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::console::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::console::auth::see_other("/"));
        }
        let form = match crate::console::auth::read_form(request).await {
            Ok(form) => form,
            Err(response) => return Ok(response),
        };
        let asked =
            crate::features::backups::set_bucket_credentials_request::SetBucketCredentialsRequest {
                access_key_id: crate::console::auth::field(&form, "access_key_id").to_string(),
                secret_access_key: crate::console::auth::field(&form, "secret_access_key")
                    .to_string(),
                region: crate::console::auth::field(&form, "region").to_string(),
                endpoint: crate::console::auth::field(&form, "endpoint").to_string(),
            };
        match crate::features::backups::node_user_set_bucket_credentials::run(
            &self.state.database,
            &self.state.config,
            &account,
            &asked,
        )
        .await
        {
            Ok(()) => Ok(crate::console::auth::see_other(here)),
            Err(reason) => Ok(crate::console::auth::back_with_error(here, &reason)),
        }
    }

    /// Take the credential out of the file.
    #[post("/node/backups/credentials/forget")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn forget_bucket_credentials(&self, _request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::console::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::console::auth::see_other("/"));
        }
        if let Err(reason) = crate::features::backups::node_user_forget_bucket_credentials::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            return Ok(crate::console::auth::back_with_error(here, &reason));
        }
        Ok(crate::console::auth::see_other(here))
    }

    /// Take one now.
    ///
    /// Started and not awaited: a backup is minutes of `pg_basebackup`
    /// and file copying, and a browser waiting on that times out
    /// somewhere in the middle — leaving the operator with no page and a
    /// backup still running. The page it comes back to says one is
    /// running, and how the last one went.
    #[post("/node/backups/now")]
    #[raw]
    #[middleware(SessionMiddleware)]
    async fn back_up_now(&self, _request: Request) -> RestResult<Response> {
        let here = &NodeTab::Backup.path();
        let Some(account) = signed_in(&self.auth) else {
            return Ok(crate::console::auth::see_other("/sign-in"));
        };
        if !crate::authz::node_user::manages_the_machine(&account) {
            return Ok(crate::console::auth::see_other("/"));
        }
        // Refused rather than queued. Two of these is two copies of every
        // database at once on a machine with one core, and a second click
        // on a button that has not answered yet is the ordinary way to
        // ask for that.
        match crate::features::backups::node_user_back_up_now::run(
            &self.state.database,
            &self.state.config,
            &account,
        )
        .await
        {
            Ok(()) => Ok(crate::console::auth::see_other(here)),
            Err(reason) => Ok(crate::console::auth::back_with_error(here, &reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::tests::Console;

    /// The tab Jorge asked for, and what had to move onto it.
    ///
    /// The write-ahead log switch was on Capabilities, as a third thing
    /// the node either does or does not do. It belongs beside the
    /// schedule: point-in-time recovery is how far a restore *reaches*
    /// from a backup, and with no backup to anchor it the log is disk
    /// nothing can use.
    #[tokio::test]
    async fn the_backup_tab_carries_the_plan_and_the_log_switch() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;

        assert!(body.contains(r#"action="/node/backups""#), "{body}");
        assert!(
            body.contains(r#"name="archiving""#),
            "the log switch is here now: {body}"
        );
        assert!(
            body.contains(r#"action="/node/backups/now""#),
            "and the button that takes one now: {body}"
        );
        // The island hides what does not apply; it does not build the
        // form. Every field is in the HTML the node produced, which is
        // what makes this page work with scripting off.
        assert!(body.contains(r#"data-island="fields""#), "{body}");
        assert!(body.contains(r#"data-when="cadence=weekly""#), "{body}");
        crate::console::tests::conditions_name_real_controls(&body);

        // And it is gone from where it was. A switch on two tabs is a
        // switch somebody saves twice, with two different answers.
        let allows = console
            .harness
            .get(&NodeTab::Allows.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            !allows.contains(r#"name="archiving""#),
            "the log switch is still on Capabilities: {allows}"
        );
    }

    /// A plan survives the form and comes back selected.
    ///
    /// The second half is the half that has been wrong here before:
    /// `selected` is read by *presence*, so a value rendered as
    /// `selected=(false)` selects — which is how a selector came to name
    /// the wrong project. Every option on this form branches.
    #[tokio::test]
    async fn a_backup_plan_is_stored_and_comes_back_selected() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let saved = console
            .harness
            .post("/node/backups")
            .header("cookie", &cookie)
            .form(&[
                ("destination", "/mnt/backups"),
                ("cadence", "weekly"),
                ("hour", "4"),
                ("weekday", "2"),
                ("keep_days", "30"),
            ])
            .send()
            .await;
        assert_eq!(saved.header("location"), Some("/node/backup"));

        let plan = crate::node::backups::plan(&console.database).await;
        assert_eq!(plan.destination.as_deref(), Some("/mnt/backups"));
        assert_eq!(plan.cadence, crate::node::backups::Cadence::Weekly);
        assert_eq!(plan.hour, 4);
        assert_eq!(plan.weekday, 2);
        assert_eq!(plan.keep_days, 30);

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains(r#"value="weekly" selected"#), "{body}");
        assert!(body.contains(r#"value="4" selected"#), "the hour: {body}");
        assert!(body.contains(r#"value="2" selected"#), "the day: {body}");
        assert!(body.contains(r#"value="30""#), "the window: {body}");
    }

    /// A destination that cannot work is refused when it is typed.
    ///
    /// Not when the run comes round. A schedule saved against
    /// `sftp://…` — one letter from a scheme this understands — is a job
    /// that fails at three in the morning for a reason that was
    /// available the moment somebody typed it.
    #[tokio::test]
    async fn a_destination_this_node_cannot_write_to_is_refused() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($destination:expr) => {{
                let response = console
                    .harness
                    .post("/node/backups")
                    .header("cookie", &cookie)
                    .form(&[
                        ("destination", $destination),
                        ("cadence", "daily"),
                        ("hour", "3"),
                        ("weekday", "6"),
                        ("keep_days", "7"),
                    ])
                    .send()
                    .await;
                response.header("location").unwrap_or_default().to_string()
            }};
        }

        let refused = save!("sftp://host/backups");
        assert!(refused.contains("error="), "{refused}");
        // A bucket with no credentials on this node: the same answer, and
        // it names the file to put them in rather than the failure.
        let no_credentials = save!("s3://somebodys-bucket/nodes");
        assert!(no_credentials.contains("error="), "{no_credentials}");
        assert!(
            no_credentials.contains("card+below"),
            "and says where they go — the card on this tab, not an SSH session: {no_credentials}"
        );
        assert_eq!(
            crate::node::backups::plan(&console.database).await,
            crate::node::backups::Plan::default(),
            "a refused form must store nothing"
        );

        // And a window of no days, which would delete each backup as
        // soon as it was taken.
        let response = console
            .harness
            .post("/node/backups")
            .header("cookie", &cookie)
            .form(&[
                ("destination", ""),
                ("cadence", "daily"),
                ("hour", "3"),
                ("weekday", "6"),
                ("keep_days", "0"),
            ])
            .send()
            .await;
        let refused = response.header("location").unwrap_or_default().to_string();
        assert!(refused.contains("error="), "{refused}");
    }

    /// The switch reaches the row, and the databases are told.
    ///
    /// The telling is what makes it more than a checkbox: `archive_mode`
    /// is a postmaster setting written into the container's arguments, so
    /// without a redeployment the switch reads on, the page says on, and
    /// nothing archives.
    #[tokio::test]
    async fn the_log_switch_is_saved_from_the_backup_form() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($fields:expr) => {
                console
                    .harness
                    .post("/node/backups")
                    .header("cookie", &cookie)
                    .form($fields)
                    .send()
                    .await
            };
        }

        // A checkbox arrives only when ticked, so its absence is the
        // answer rather than a missing field.
        save!(&[("cadence", ""), ("keep_days", "7")]);
        assert!(!crate::node::settings::archiving(&console.database).await);

        save!(&[("cadence", ""), ("keep_days", "7"), ("archiving", "1")]);
        assert!(crate::node::settings::archiving(&console.database).await);
    }

    /// What is on the disk, and whose it is.
    ///
    /// A shared backup root holds several nodes' copies — that is what
    /// makes one bucket the answer for a network rather than for a
    /// machine — so a row about another node has to say so rather than
    /// read as this one's.
    #[tokio::test]
    async fn a_backup_on_the_disk_says_whose_it_is() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;

        let write = |node: &str, at: i64, volumes: usize| {
            let into = console
                .data_dir
                .join("backups")
                .join("nodes")
                .join(node)
                .join(at.to_string());
            std::fs::create_dir_all(&into).expect("a backup directory");
            let manifest = serde_json::json!({
                "format": 1,
                "taken_by": "wabot-deploy 0.0.0-test",
                "taken_at": at,
                "node_id": node,
                "node_name": format!("{node}.example"),
                "volumes": (0..volumes).map(|index| serde_json::json!({
                    "container": format!("demo.orders-{index}"),
                    "bytes": 1024,
                    "how": "Consistent",
                })).collect::<Vec<_>>(),
                "images": [],
            });
            std::fs::write(
                into.join("manifest.json"),
                serde_json::to_vec(&manifest).expect("a manifest"),
            )
            .expect("write it");
        };

        let me = crate::network::me(&console.database)
            .await
            .expect("read")
            .expect("this node");
        write(&me.id, 1_700_000_000_000, 2);
        write("some-other-node", 1_600_000_000_000, 1);

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("2 volume(s)"), "{body}");
        assert!(body.contains("this node"), "{body}");
        assert!(
            body.contains("some-other-node.example"),
            "the other node's row names it: {body}"
        );
    }

    /// A bucket is configured from the console, and the secret is not in
    /// the database.
    ///
    /// Jorge asked for the mechanism — configuring one was an SSH session
    /// and a text editor — and the answer is *which file*, not who writes
    /// it: a key that can read every backup in the network must not be
    /// inside the thing being backed up.
    #[tokio::test]
    async fn a_bucket_credential_is_stored_in_the_file_and_not_the_database() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        let path = console.data_dir.join("config.toml");

        let saved = console
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", "s3cr3t-from-the-provider"),
                ("region", "us-east-1"),
                ("endpoint", "https://s3.us-west-004.backblazeb2.com"),
            ])
            .send()
            .await;
        assert_eq!(saved.header("location"), Some("/node/backup"));

        let file = std::fs::read_to_string(&path).expect("the config file");
        assert!(file.contains("AKIAEXAMPLE"), "{file}");
        assert!(file.contains("s3cr3t-from-the-provider"), "{file}");

        // And nowhere near the database. Read as bytes because the
        // question is "is that string anywhere in this file", not "is
        // there a row" — a secret that leaked into any column at all
        // would travel in every backup.
        let db = std::fs::read(console.data_dir.join("db").join("node.db")).unwrap_or_default();
        assert!(
            !String::from_utf8_lossy(&db).contains("s3cr3t-from-the-provider"),
            "the secret must not be in the node's own database"
        );

        // The page shows that there is one, and never what it is.
        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(
            body.contains("AKIAEXAMPLE"),
            "the key id is not a secret: {body}"
        );
        assert!(
            !body.contains("s3cr3t-from-the-provider"),
            "the secret is never rendered back"
        );
        assert!(
            body.contains(r#"action="/node/backups/credentials/forget""#),
            "and it can be removed: {body}"
        );
    }

    /// Changing the region does not mean retyping the secret.
    ///
    /// The field renders empty whatever is stored — it is never shown
    /// again — so empty on save has to mean "keep it". Reading it as
    /// "clear it" would wipe the credential of everybody who came to fix
    /// a region, and they would find out at the next backup.
    #[tokio::test]
    async fn an_empty_secret_keeps_the_one_that_is_stored() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        macro_rules! save {
            ($secret:expr, $region:expr) => {
                console
                    .harness
                    .post("/node/backups/credentials")
                    .header("cookie", &cookie)
                    .form(&[
                        ("access_key_id", "AKIAEXAMPLE"),
                        ("secret_access_key", $secret),
                        ("region", $region),
                        ("endpoint", ""),
                    ])
                    .send()
                    .await
            };
        }

        save!("kept-across-the-edit", "us-east-1");
        let moved = save!("", "eu-central-1");
        assert_eq!(moved.header("location"), Some("/node/backup"));

        let stored = console.config.s3().expect("still a credential");
        assert_eq!(stored.secret_access_key, "kept-across-the-edit");
        assert_eq!(stored.region, "eu-central-1");

        // With nothing stored there is nothing to keep, and the refusal
        // says so rather than writing a credential with no secret in it.
        let bare = Console::new().await;
        let bare_cookie = bare.signed_in().await;
        let refused = bare
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &bare_cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", ""),
                ("region", "us-east-1"),
                ("endpoint", ""),
            ])
            .send()
            .await;
        assert!(
            refused
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "{:?}",
            refused.header("location")
        );
    }

    /// Forgetting the credential while the backups go to a bucket is
    /// refused.
    ///
    /// The button is disabled too, but that is a courtesy — the check
    /// that counts is here. A plan that cannot be carried out fails at
    /// the next slot, which is hours after the click that caused it.
    #[tokio::test]
    async fn a_credential_in_use_is_not_forgotten_by_accident() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        console
            .harness
            .post("/node/backups/credentials")
            .header("cookie", &cookie)
            .form(&[
                ("access_key_id", "AKIAEXAMPLE"),
                ("secret_access_key", "a-secret"),
                ("region", "us-east-1"),
                ("endpoint", ""),
            ])
            .send()
            .await;
        // The destination is written directly: saving it through the form
        // would ask the bucket, and there is no bucket in a test.
        crate::node::backups::store(
            &console.database,
            &crate::node::backups::Plan {
                destination: Some("s3://somebodys-bucket/nodes".into()),
                ..crate::node::backups::Plan::default()
            },
        )
        .await
        .expect("store the plan");

        let refused = console
            .harness
            .post("/node/backups/credentials/forget")
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(
            refused
                .header("location")
                .unwrap_or_default()
                .contains("error="),
            "{:?}",
            refused.header("location")
        );
        assert!(console.config.s3().is_some(), "still stored");

        // Pointed somewhere else, it goes.
        crate::node::backups::store(&console.database, &crate::node::backups::Plan::default())
            .await
            .expect("store the plan");
        console
            .harness
            .post("/node/backups/credentials/forget")
            .header("cookie", &cookie)
            .send()
            .await;
        assert!(console.config.s3().is_none(), "and it is gone");
    }

    /// A failure is a value somebody can act on.
    ///
    /// A destination that has been refusing for a week is invisible in a
    /// journal nobody reads, so the reason is on the row — and from there
    /// on the page and in the attention card.
    #[tokio::test]
    async fn the_last_failure_is_on_the_page() {
        let console = Console::new().await;
        let cookie = console.signed_in().await;
        crate::node::backups::record(
            &console.database,
            crate::clock::now_ms(),
            &Err("ssh: connect to host backups.example port 22: no route to host".into()),
        )
        .await
        .expect("record");

        let body = console
            .harness
            .get(&NodeTab::Backup.path())
            .header("cookie", &cookie)
            .send()
            .await
            .body;
        assert!(body.contains("no route to host"), "{body}");
    }
}
