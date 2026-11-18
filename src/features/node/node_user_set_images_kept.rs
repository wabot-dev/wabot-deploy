use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::features::node::set_images_kept_request::SetImagesKeptRequest;

/// Collected at once, so a number lowered gives its disk back now rather
/// than at the next push.
pub async fn run(
    database: &Arc<SqliteDatabase>,
    account: &Account,
    request: &SetImagesKeptRequest,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }
    if request.kept == Some(0) {
        return Err("at least one: the image a service runs is always kept".into());
    }

    crate::infra::node::settings::set_images_kept(database, request.kept)
        .await
        .map_err(|error| error.to_string())?;

    let database = database.clone();
    tokio::spawn(async move { crate::infra::deploy::surplus::sweep(&database).await });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::roles::NodeRole;

    async fn database() -> Arc<SqliteDatabase> {
        Arc::new(crate::infra::db::open_in_memory().await.expect("open"))
    }

    fn account(role: NodeRole) -> Account {
        Account {
            id: "ac-1".into(),
            username: "someone".into(),
            role,
            theme: crate::domain::values::theme::Theme::System,
            language: crate::domain::values::language::Language::En,
            tenant_id: None,
        }
    }

    #[tokio::test]
    async fn the_number_is_stored_and_empty_goes_back_to_the_default() {
        let database = database().await;
        let admin = account(NodeRole::Admin);

        run(&database, &admin, &SetImagesKeptRequest { kept: Some(5) })
            .await
            .expect("stored");
        assert_eq!(
            crate::infra::node::settings::images_kept(&database).await,
            Some(5)
        );

        run(&database, &admin, &SetImagesKeptRequest { kept: None })
            .await
            .expect("cleared");
        assert_eq!(
            crate::infra::node::settings::images_kept(&database).await,
            None
        );
    }

    #[tokio::test]
    async fn none_is_refused_and_so_is_anybody_but_an_administrator() {
        let database = database().await;

        let refused = run(
            &database,
            &account(NodeRole::Admin),
            &SetImagesKeptRequest { kept: Some(0) },
        )
        .await
        .expect_err("refused");
        assert!(refused.contains("at least one"), "{refused}");

        run(
            &database,
            &account(NodeRole::Member),
            &SetImagesKeptRequest { kept: Some(3) },
        )
        .await
        .expect_err("not theirs to set");
        assert_eq!(
            crate::infra::node::settings::images_kept(&database).await,
            None
        );
    }
}
