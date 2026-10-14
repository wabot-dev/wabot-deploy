use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::port::Port;
use crate::domain::entities::service::Service;
use crate::infra::deploy::dns;
use crate::infra::repositories::ports;

pub struct AddServicePort<'a> {
    pub service: &'a Service,
    pub asked: &'a crate::features::services::add_service_port_request::AddServicePortRequest,
}

impl AddServicePort<'_> {
    /// **Nothing asks an authority for a name that does not point here.**
    /// The check is the same one the renewal loop makes before it spends
    /// anything; this is what gives an answer in the form instead of two
    /// failed validations a day against an authority that locks the
    /// account at five an hour.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<Port, String> {
        let (hostname, edges) = match self.asked.https {
            false => (None, Vec::new()),
            true => {
                if self.asked.hostname.is_empty() {
                    return Err("serving over HTTPS needs a hostname".into());
                }
                let parsed = crate::domain::values::hostname::Hostname::parse(&self.asked.hostname);
                if parsed.is_wildcard() {
                    return Err(crate::domain::values::hostname::WILDCARD_REFUSED.into());
                }
                let hostname = parsed.as_str().to_string();

                let references = dns::references(database).await?;
                if references.is_empty() {
                    let named = crate::infra::node::settings::domain(database)
                        .await
                        .is_some();
                    return Err(match named {
                        true => "no node on this network answers for hostnames — this one has \
                                 that turned off in its capabilities"
                            .into(),
                        false => "no node on this network has a domain of its own, so there is \
                                  nothing to check that name against — give this node one on its \
                                  settings page, or join a node that has one"
                            .into(),
                    });
                }

                let outcome = dns::resolves_to(&hostname, &references).await;
                if !outcome.ok() {
                    return Err(outcome.explain(&hostname));
                }
                (Some(hostname), outcome.nodes())
            }
        };

        ports::create(
            database,
            &self.service.id,
            self.asked.container_port,
            self.asked.publish,
            hostname.as_deref(),
            &edges,
        )
        .await
        .map_err(|error| error.to_string())
    }
}
