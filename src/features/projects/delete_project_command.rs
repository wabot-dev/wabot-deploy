use wabot::prelude::Container;
use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::features::dns::delete_zone_command::DeleteZone;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::projects;
use crate::infra::repositories::services;

pub struct DeleteProject<'a> {
    pub project: &'a Project,
    /// The project's own slug, typed. Checked here rather than trusted
    /// from the form: `required` is a courtesy the browser extends, and
    /// a request can be constructed by hand.
    pub confirm: &'a str,
}

impl DeleteProject<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        container: &Container,
        deployer: &Deployer,
    ) -> Result<(), String> {
        if self.confirm.trim() != self.project.slug {
            return Err("type the project's name to confirm the deletion".into());
        }

        // **Every service is taken down first, one at a time.** This used
        // to be the row deletion alone, and `ON DELETE CASCADE` took the
        // service rows with it — which is not the same as taking the
        // *services*. Nothing stopped a container and nothing removed a
        // volume, and the container id is derived from the rows, so
        // losing the rows loses the handle for ever.
        let all = services::all(database, Some(&self.project.id))
            .await
            .unwrap_or_default();
        for service in &all {
            deployer.tear_down(self.project, service).await;
            deployer.discard_storage(self.project, service).await;
        }

        // The zones the same way, for the same reason: the cascade would
        // take the rows and leave every other node that answers for them
        // answering, with nobody left to tell it to stop.
        let zones = crate::infra::repositories::zones::of_project(database, &self.project.id)
            .await
            .map_err(|error| error.to_string())?;
        for zone in &zones {
            DeleteZone { zone }.run(database, container).await?;
        }

        projects::delete(database, &self.project.id)
            .await
            .map_err(|error| error.to_string())?;
        deployer.sync_routes().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::domain::values::capability::Capability;
    use crate::domain::values::zone_name::ZoneName;
    use crate::infra::config::Config;
    use crate::infra::network::errand::{self, Kind};
    use crate::infra::repositories::zones;

    /// The cascade would have taken the zone's rows and nothing else: every
    /// other node answering for it would have gone on answering, with
    /// nobody left on this side to tell it to stop.
    #[tokio::test]
    async fn deleting_a_project_tells_every_node_answering_for_its_zones_to_stop() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::node::settings::set_domain(&database, Some("node-0.example.net"))
            .await
            .expect("domain");
        let me = crate::infra::network::ensure_self(&database, &Config::default())
            .await
            .expect("self")
            .id;
        crate::infra::network::save(
            &database,
            &crate::infra::network::Node {
                update_note: None,
                version: None,
                wanted_version: None,
                usage: None,
                draining_at: None,
                capacity: None,
                id: "nd-peer".into(),
                name: "node-1.example.net".into(),
                kind: crate::infra::network::Kind::Public,
                endpoint: Some("node-1.example.net:443".into()),
                public_key: None,
                overlay_ip: Some("10.42.0.8".into()),
                is_self: false,
                last_seen_at: None,
                allows: vec![Capability::Dns],
                ca_pem: None,
            },
        )
        .await
        .expect("a peer that answers DNS for this node");

        let site = projects::create(&database, "Site").await.expect("project");
        let zone = zones::create(
            &database,
            zones::NewZone {
                name: &ZoneName::parse("teresaasprilla.com").expect("zone"),
                project: &site,
                servers: &[me, "nd-peer".into()],
            },
        )
        .await
        .expect("zone");
        crate::infra::dns::dispatch::tell_servers(&database, &zone, &[]).await;

        let deployer = Deployer::new(Arc::new(database.clone()), &Config::default());
        DeleteProject {
            project: &site,
            confirm: "site",
        }
        .run(&database, &Container::new(), &deployer)
        .await
        .expect("deleted");

        assert!(zones::all(&database).await.expect("zones").is_empty());
        let last = errand::all(&database)
            .await
            .expect("errands")
            .into_iter()
            .find(|record| record.kind == Kind::Zone)
            .expect("a zone errand");
        assert_eq!(last.node_id, "nd-peer");
        let payload: errand::Zone = serde_json::from_value(last.payload).expect("a zone");
        assert!(payload.withdrawn, "the peer was not told to stop");
    }
}
