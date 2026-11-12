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
                if parsed.is_wildcard() && parsed.wildcard_base().is_none() {
                    return Err(crate::domain::values::hostname::WILDCARD_REFUSED.into());
                }
                let hostname = parsed.as_str().to_string();
                let probed = match parsed.wildcard_base() {
                    Some(base) => {
                        require_zone_for(database, self.service, base).await?;
                        dns::wildcard_probe(base)
                    }
                    None => hostname.clone(),
                };

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

                let outcome = dns::resolves_to(&probed, &references).await;
                if !outcome.ok() {
                    return Err(match parsed.wildcard_base() {
                        Some(base) => format!(
                            "{} — a wildcard needs a `*` record in the zone for {base} that \
                             points at this node",
                            outcome.explain(&probed)
                        ),
                        None => outcome.explain(&hostname),
                    });
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

async fn require_zone_for(
    database: &SqliteDatabase,
    service: &Service,
    base: &str,
) -> Result<(), String> {
    let zone = crate::infra::repositories::zones::holding(database, &service.project_id, base)
        .await
        .map_err(|error| error.to_string())?;
    match zone {
        Some(_) => Ok(()),
        None => Err(format!(
            "this project has no DNS zone holding {base} — a wildcard certificate is proved in \
             DNS, so add the zone to the project first"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::services::add_service_port_request::AddServicePortRequest;

    #[tokio::test]
    async fn a_wildcard_needs_a_zone_of_the_services_own_project() {
        let database = crate::infra::db::open_in_memory().await.expect("open");
        let project = crate::infra::repositories::projects::create(&database, "demo")
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
        let asked = AddServicePortRequest {
            container_port: 80,
            https: true,
            hostname: "*.apps.example.com".into(),
            ..Default::default()
        };

        let refused = AddServicePort {
            service: &service,
            asked: &asked,
        }
        .run(&database)
        .await;

        assert!(
            matches!(&refused, Err(reason) if reason.contains("no DNS zone")),
            "{refused:?}"
        );
    }
}
