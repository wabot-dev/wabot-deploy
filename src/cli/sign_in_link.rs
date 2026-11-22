//! `wabot-deploy sign-in-link` — a way back into a console.
//!
//! ## Why this exists
//!
//! Nobody here has a password to forget any more, but somebody can lose
//! every device that could let a new one in, and a node opened by its
//! address can make no passkey at all. The setup token refuses once an
//! account exists — that is what makes it safe to print — so without this
//! the only route back was reinstalling.
//!
//! ## Why it is not a hole
//!
//! Whoever runs this already has root on the box: the binary, the
//! database, every container and every secret in them. What changes is
//! only that recovery is obvious instead of surgery, and that its owner
//! reads in their activity that it was issued here.
//!
//! ## Why one link per account
//!
//! A name is unique in one console, not on the node: the node's `jorge`
//! and an organisation's `jorge` are two people. Each gets the link to
//! the console they belong to.

use crate::infra::accounts::links::{self, Purpose};
use crate::infra::config::Config;

pub async fn run(config: Config, username: &str) -> anyhow::Result<i32> {
    let database = crate::infra::db::open(&config.database_path()).await?;
    let found = crate::infra::accounts::named(&database, username).await?;
    if found.is_empty() {
        database.close().await?;
        println!("no account here is called {username}");
        return Ok(1);
    }

    let mut issued = Vec::new();
    for account in &found {
        let token = links::issue(&database, &account.id, Purpose::SignIn, None).await?;
        let base = crate::infra::node::base_url(
            &database,
            config.edge.https_port,
            account.tenant_id.as_deref(),
        )
        .await;
        let place = match account.tenant_id.as_deref() {
            Some(id) => crate::infra::repositories::tenants::find(&database, id)
                .await
                .ok()
                .flatten()
                .map(|tenant| format!("in {}", tenant.name))
                .unwrap_or_else(|| "in an organisation".into()),
            None => "on this node".into(),
        };
        issued.push((
            account.username.clone(),
            place,
            format!("{base}/link/{token}"),
        ));
    }
    database.close().await?;

    println!();
    for (name, place, url) in issued {
        println!("  {name}, {place}:");
        println!();
        println!("  {url}");
        println!();
    }
    println!("  Each works once, within fifteen minutes, and makes a passkey on");
    println!("  the device it is opened on. Nothing else was touched.");
    println!();
    Ok(0)
}
