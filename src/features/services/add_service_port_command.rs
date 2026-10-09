use wabot::sqlite::SqliteDatabase;

use crate::config::Config;
use crate::deploy::dns;
use crate::entities::port::Port;
use crate::entities::service::Service;
use crate::repositories::ports;

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
    pub async fn run(&self, database: &SqliteDatabase, config: &Config) -> Result<Port, String> {
        let (hostname, edges) = match self.asked.https {
            false => (None, Vec::new()),
            true => {
                if self.asked.hostname.is_empty() {
                    return Err("serving over HTTPS needs a hostname".into());
                }
                let hostname = crate::values::hostname::Hostname::parse(&self.asked.hostname)
                    .as_str()
                    .to_string();

                let references = dns::references(database, config).await?;
                if references.is_empty() {
                    return Err(
                        "no node on this network has a domain of its own, so there is \
                                nothing to check that name against — give this node one on its \
                                settings page, or join a node that has one"
                            .into(),
                    );
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
