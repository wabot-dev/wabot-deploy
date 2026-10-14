use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::infra::deploy::dns;
use crate::infra::repositories::edges;

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
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Vec<String>, String> {
        if self.hostname.is_empty() {
            return Err("that names no hostname".into());
        }
        // An edge row is what the renewal loop orders for, so one for a
        // name no port of this service has would be a certificate asked
        // for a name nothing serves — on the strength of a form field.
        let ours = crate::infra::repositories::ports::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .any(|port| port.hostname.as_deref() == Some(self.hostname));
        if !ours {
            return Err("that is not one of this service's names".into());
        }

        let references: Vec<dns::Reference> = dns::references(database)
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
