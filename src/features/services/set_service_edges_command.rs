use wabot::sqlite::SqliteDatabase;

use crate::config::Config;
use crate::deploy::dns;
use crate::entities::service::Service;
use crate::repositories::edges;

/// Which public nodes answer for one of this service's names.
///
/// **A node dropped from the list is told too**, with an empty upstream
/// list: it keeps answering for the name, certificate and all, until
/// something says otherwise. `edges::set` returns who was dropped so the
/// caller cannot forget.
pub struct SetServiceEdges<'a> {
    pub service: &'a Service,
    pub hostname: &'a str,
    pub chosen: &'a [String],
}

impl SetServiceEdges<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        config: &Config,
    ) -> Result<Vec<String>, String> {
        if self.hostname.is_empty() {
            return Err("that names no hostname".into());
        }

        let references: Vec<dns::Reference> = dns::references(database, config)
            .await?
            .into_iter()
            .filter(|reference| self.chosen.contains(&reference.node_id))
            .collect();

        // A node the name does not reach would answer for traffic that
        // never arrives.
        if !self.chosen.is_empty() && !references.is_empty() {
            let reached = dns::resolves_to(self.hostname, &references).await.nodes();
            let missing: Vec<String> = references
                .iter()
                .filter(|reference| !reached.contains(&reference.node_id))
                .map(|reference| reference.node_name.clone())
                .collect();
            if !missing.is_empty() {
                return Err(format!(
                    "{} does not resolve to {} — a node that the name does not reach would \
                     answer for traffic that never arrives. Point the record at it and try again.",
                    self.hostname,
                    missing.join(", ")
                ));
            }
        }

        edges::set(database, &self.service.id, self.hostname, self.chosen)
            .await
            .map_err(|error| error.to_string())
    }
}
