//! `wabot-deploy setup-token` — mint one, print it, forget it.

use crate::infra::config::Config;

pub async fn run(config: Config) -> anyhow::Result<i32> {
    let database = crate::infra::db::open(&config.database_path()).await?;
    let url = url(&database, &config).await;

    if crate::infra::accounts::any_account(&database).await? {
        println!("This node already has an administrator. Sign in at {url}.");
        println!();
        println!("A setup token cannot create a second one — that is the point of it.");
        database.close().await?;
        // Not an error: asking is reasonable, and the answer is no.
        return Ok(0);
    }

    let token = crate::infra::accounts::issue_setup_token(&database).await?;
    database.close().await?;

    print(&url, &token);
    Ok(0)
}

/// Print the token and what to do with it.
///
/// Shared with `install`, so the two never drift into telling an
/// operator two different things.
pub fn print(url: &str, token: &str) {
    println!();
    println!("  Create the administrator account at:");
    println!("  {url}/setup");
    println!();
    println!("  setup token: {token}");
    println!();
    println!("  It works once, and it expires in 24 hours.");
    println!("  `wabot-deploy setup-token` issues another.");
}

/// Where the console answers, from wherever the operator is.
///
/// **Not `localhost`.** That was the fallback, and the reasoning written
/// down for it — "from the machine itself localhost is that URL" — is
/// wrong about who is reading. Somebody installing on a box over SSH is
/// on their laptop, and `localhost` is the single address that is
/// certain not to work for them. Reported by Jorge, installing on a
/// fresh node.
///
/// So: the domain if there is one, and otherwise the address this
/// machine would use to reach the world — which on a cloud box is its
/// public one, and on a machine behind NAT is a private address that is
/// at least reachable from the network the operator is probably on.
/// `localhost` survives only for a machine with no route at all, where
/// nothing better is knowable and the reader really is sitting at it.
///
/// The port is only spelled out when it is not 443: a URL with `:443` on
/// it is one somebody will copy into a browser and wonder about.
///
/// The stored domain, not the file's: the console writes the row, and a
/// node renamed there was told to sign in at the address it had before.
/// `passwd` printed `https://<this node>:443` under a comment saying it
/// spoke the way this does. Found on a VM node.
pub async fn url(database: &wabot::sqlite::SqliteDatabase, config: &Config) -> String {
    address(
        crate::infra::node::settings::domain(database)
            .await
            .as_deref(),
        config.edge.https_port,
    )
}

fn address(domain: Option<&str>, https_port: u16) -> String {
    format!(
        "https://{}",
        crate::infra::node::authority(domain, https_port)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_port_is_not_spelled_out() {
        assert_eq!(address(Some("node.example"), 443), "https://node.example");
        assert_eq!(
            address(Some("node.example"), 8443),
            "https://node.example:8443"
        );
    }

    /// The console's domain, not the one install wrote: a node renamed
    /// in the console told an operator to sign in at its old name.
    #[tokio::test]
    async fn the_address_is_the_one_the_console_set() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let mut config = Config::default();
        config.seed_domain(Some("installed.example".into()));
        config.edge.https_port = 443;
        crate::infra::node::settings::set_domain(&database, Some("renamed.example"))
            .await
            .expect("a domain");
        assert_eq!(url(&database, &config).await, "https://renamed.example");
    }

    /// A node with no domain still has to give somebody a URL they can
    /// open — and `localhost` is not it.
    ///
    /// This test used to assert the opposite, with a doc comment saying
    /// that from the machine itself localhost is that URL. The reasoning
    /// was wrong about who reads the line: an operator installing over
    /// SSH is on their laptop, where `localhost` is the one address
    /// guaranteed to fail. It now asserts an address somebody elsewhere
    /// could actually open.
    #[test]
    fn a_node_with_no_domain_names_an_address_somebody_else_can_open() {
        let printed = address(None, 443);
        assert!(printed.starts_with("https://"), "{printed}");

        // On a machine with a route out, that is its own address rather
        // than loopback. On one without, loopback is all there is and
        // the reader is necessarily sitting at it.
        match crate::infra::deploy::dns::outbound_address() {
            Some(address) => assert!(printed.contains(&address.to_string()), "{printed}"),
            None => assert!(printed.contains("localhost"), "{printed}"),
        }
    }
}
