//! Telling a node to stop answering for a name nobody here asks it to.
//!
//! A name stops being answered for when its rows go — a port deleted or
//! renamed, a redirect removed, a service or a project deleted — and each
//! of those used to be a place that had to remember to tell the other
//! nodes. Most did not, so a name went on being served there, with a
//! certificate renewed for it twice a day. This asks instead: what was
//! each node last told, and do the rows still say so? It runs at the end
//! of every route sync, so whatever changed the names is covered, and so
//! is a boot after a change that never got that far.

use std::collections::BTreeSet;

use wabot::sqlite::SqliteDatabase;

use crate::infra::network::errand;
use crate::infra::repositories::edges;

/// How many nodes were told to stop answering for a name.
pub async fn withdraw_unwanted(database: &SqliteDatabase) -> usize {
    let (Ok(told), Ok(wanted)) = (
        errand::names_told(database).await,
        edges::all(database).await,
    ) else {
        tracing::warn!("could not work out which names the other nodes still answer for");
        return 0;
    };
    let wanted: BTreeSet<(String, String)> = wanted
        .into_iter()
        .map(|(hostname, node)| (node, hostname))
        .collect();
    let me = crate::infra::network::me(database)
        .await
        .ok()
        .flatten()
        .map(|me| me.id);

    let mut withdrawn = 0;
    for (node, hostname) in told {
        if Some(&node) == me.as_ref() || wanted.contains(&(node.clone(), hostname.clone())) {
            continue;
        }
        match errand::withdraw_name(database, &node, &hostname).await {
            Ok(_) => {
                tracing::info!(%node, %hostname, "told a node to stop answering for a name");
                withdrawn += 1;
            }
            Err(error) => tracing::warn!(%error, %node, %hostname, "could not withdraw a name"),
        }
    }
    withdrawn
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::config::Config;

    async fn told_to_serve(database: &SqliteDatabase, node: &str, hostname: &str) {
        let payload = serde_json::to_value(errand::Edge {
            hostname: hostname.into(),
            upstreams: vec!["10.42.0.3:30000".into()],
            peers: Vec::new(),
            secret: None,
            redirect_to: None,
        })
        .expect("payload");
        errand::queue(database, node, errand::Kind::Edge, &payload)
            .await
            .expect("queued");
    }

    /// A node told to serve a name the rows no longer give it is told to
    /// stop — once — and a name the rows still give it is left alone,
    /// whatever dropped the other one.
    #[tokio::test]
    async fn a_node_still_serving_a_name_nobody_wants_from_it_is_told_to_stop_once() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let me = crate::infra::network::ensure_self(&database, &Config::default())
            .await
            .expect("self")
            .id;
        let project = crate::infra::repositories::projects::create(&database, "site")
            .await
            .expect("project");
        let service = crate::infra::repositories::services::create(
            &database,
            &project.id,
            "web",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        edges::set(&database, &service.id, "kept.example", &["nd-peer".into()])
            .await
            .expect("still wanted");

        told_to_serve(&database, "nd-peer", "kept.example").await;
        told_to_serve(&database, "nd-peer", "gone.example").await;
        told_to_serve(&database, &me, "gone.example").await;

        assert_eq!(withdraw_unwanted(&database).await, 1);
        let still: BTreeSet<(String, String)> = errand::names_told(&database)
            .await
            .expect("told")
            .into_iter()
            .collect();
        assert_eq!(
            still,
            BTreeSet::from([
                ("nd-peer".to_string(), "kept.example".to_string()),
                (me, "gone.example".to_string()),
            ]),
            "only the peer's unwanted name was withdrawn, and never this node's own"
        );
        assert_eq!(
            withdraw_unwanted(&database).await,
            0,
            "a name already withdrawn is not withdrawn again"
        );
    }
}
