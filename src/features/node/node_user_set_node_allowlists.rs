use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::node_user;
use crate::domain::entities::account::Account;
use crate::domain::values::allowlist::Door;
use crate::features::node::set_node_allowlists_request::SetNodeAllowlistsRequest;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::allowlists;

/// **A list that would shut out the person saving it is refused.** The
/// console is the way back from every other mistake on this page, so the
/// one change that removes it is the one asked about first — with the
/// address, because it is what they need to write and may not know.
pub async fn run(
    database: &SqliteDatabase,
    deployer: &Deployer,
    account: &Account,
    request: &SetNodeAllowlistsRequest,
) -> Result<(), String> {
    if !node_user::manages_the_machine(account) {
        return Err("only an administrator of this node can do that".into());
    }

    if let Some(client) = request.client {
        if !request.console.admits(client) && !crate::infra::edge::routes::is_this_machine(client) {
            return Err(format!(
                "that list does not let in {client}, the address this change came from — \
                 saving it would lock you out of this console. Add it, or a range that holds it."
            ));
        }
    }

    allowlists::set(database, &Door::Console, &request.console)
        .await
        .map_err(|error| error.to_string())?;
    allowlists::set(database, &Door::Registry, &request.registry)
        .await
        .map_err(|error| error.to_string())?;
    allowlists::set(database, &Door::Ssh, &request.ssh)
        .await
        .map_err(|error| error.to_string())?;
    deployer.sync_routes().await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::values::allowlist::Allowlist;

    fn deployer(database: &SqliteDatabase) -> Deployer {
        Deployer::new(
            std::sync::Arc::new(database.clone()),
            &crate::infra::config::Config::default(),
        )
    }

    fn asked(console: &str, client: &str) -> SetNodeAllowlistsRequest {
        SetNodeAllowlistsRequest {
            console: Allowlist::parse(console).expect("parsed"),
            registry: Allowlist::parse("0.0.0.0/0\n::/0").expect("parsed"),
            ssh: Allowlist::parse("198.51.100.0/24 the office").expect("parsed"),
            client: Some(client.parse().expect("an address")),
        }
    }

    #[tokio::test]
    async fn a_console_list_that_shuts_out_who_saves_it_is_refused_with_their_address() {
        let world = crate::features::fixtures::organisation().await;
        let deployer = deployer(&world.database);

        let refused = run(
            &world.database,
            &deployer,
            &world.operator,
            &asked("203.0.113.0/24 office", "198.51.100.7"),
        )
        .await
        .expect_err("it would lock them out");
        assert!(refused.contains("198.51.100.7"), "{refused}");
        assert!(
            allowlists::of(&world.database, &Door::Console)
                .await
                .expect("read")
                .is_everyone(),
            "nothing was saved"
        );

        run(
            &world.database,
            &deployer,
            &world.operator,
            &asked("203.0.113.0/24 office", "203.0.113.9"),
        )
        .await
        .expect("a range that holds them");
        assert_eq!(
            allowlists::of(&world.database, &Door::Console)
                .await
                .expect("read")
                .text(),
            "203.0.113.0/24 office"
        );
        assert_eq!(
            allowlists::of(&world.database, &Door::Ssh)
                .await
                .expect("read")
                .text(),
            "198.51.100.0/24 the office",
            "and SSH, saved with them"
        );
    }

    /// The way back for an operator who wants nobody else in: a shell on the
    /// machine, where the console is reached from the machine itself.
    #[tokio::test]
    async fn the_machine_itself_may_close_the_console_to_everybody_else() {
        let world = crate::features::fixtures::organisation().await;
        run(
            &world.database,
            &deployer(&world.database),
            &world.operator,
            &asked("", "127.0.0.1"),
        )
        .await
        .expect("from the machine");
        assert!(allowlists::of(&world.database, &Door::Console)
            .await
            .expect("read")
            .is_empty());
    }

    #[tokio::test]
    async fn only_an_administrator_of_the_machine_sets_them() {
        let world = crate::features::fixtures::organisation().await;
        for account in [&world.admin, &world.member] {
            run(
                &world.database,
                &deployer(&world.database),
                account,
                &asked("0.0.0.0/0\n::/0", "127.0.0.1"),
            )
            .await
            .expect_err("an organisation's administrator is not the machine's");
        }
    }
}
