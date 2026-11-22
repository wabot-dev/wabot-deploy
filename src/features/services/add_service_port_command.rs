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
                    return Err(crate::domain::values::hostname::WILDCARD_SHAPE.into());
                }
                let hostname = parsed.as_str().to_string();
                crate::infra::repositories::project_domain::refuse_a_foreign_name(
                    database,
                    &self.service.project_id,
                    &hostname,
                )
                .await?;
                let probed = match parsed.wildcard_base() {
                    Some(base) => {
                        if self.asks_an_authority(database).await {
                            require_zone_for(database, self.service, base).await?;
                        }
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

    async fn asks_an_authority(&self, database: &SqliteDatabase) -> bool {
        !self.asked.without_authority && crate::infra::node::settings::acme_enabled(database).await
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
            "this project has no DNS zone holding {base} — Let's Encrypt proves a wildcard in \
             DNS, so add the zone to the project first, or choose a certificate signed here or \
             read from files"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::services::add_service_port_request::AddServicePortRequest;

    async fn service() -> (SqliteDatabase, Service) {
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
        (database, service)
    }

    async fn wildcard(
        database: &SqliteDatabase,
        service: &Service,
        without_authority: bool,
    ) -> Result<Port, String> {
        let asked = AddServicePortRequest {
            container_port: 80,
            https: true,
            hostname: "*.apps.example.com".into(),
            without_authority,
            ..Default::default()
        };
        AddServicePort {
            service,
            asked: &asked,
        }
        .run(database)
        .await
    }

    #[tokio::test]
    async fn a_wildcard_from_lets_encrypt_needs_a_zone_of_the_services_own_project() {
        let (database, service) = service().await;

        let refused = wildcard(&database, &service, false).await;

        assert!(
            matches!(&refused, Err(reason) if reason.contains("no DNS zone")),
            "{refused:?}"
        );
    }

    #[tokio::test]
    async fn a_wildcard_signed_here_or_read_from_files_needs_no_zone() {
        let (database, service) = service().await;

        let outcome = wildcard(&database, &service, true).await;

        assert!(
            !matches!(&outcome, Err(reason) if reason.contains("zone")),
            "{outcome:?}"
        );
    }

    async fn a_tenants_service(organisation_domain: Option<&str>) -> (SqliteDatabase, Service) {
        use crate::infra::repositories::{projects, services, tenants};
        let database = crate::infra::db::open_in_memory().await.expect("open");
        crate::infra::node::settings::set_domain(&database, Some("node.example.com"))
            .await
            .expect("a domain");
        let acme = tenants::create(&database, "Acme").await.expect("tenant");
        tenants::set_domain(&database, &acme.id, organisation_domain)
            .await
            .expect("its domain");
        let globex = tenants::create(&database, "Globex").await.expect("tenant");
        tenants::set_domain(&database, &globex.id, Some("globex.com"))
            .await
            .expect("its domain");
        let project = projects::create_for(&database, "shop", Some(&acme.id))
            .await
            .expect("project");
        let service = services::create(
            &database,
            &project.id,
            "web",
            "docker.io/library/nginx:alpine",
            &[],
        )
        .await
        .expect("service");
        (database, service)
    }

    async fn named(database: &SqliteDatabase, service: &Service, hostname: &str) -> String {
        let asked = AddServicePortRequest {
            container_port: 80,
            https: true,
            hostname: hostname.into(),
            ..Default::default()
        };
        AddServicePort {
            service,
            asked: &asked,
        }
        .run(database)
        .await
        .err()
        .unwrap_or_default()
    }

    /// A tenant's page under the node's domain is the same site as the
    /// node's console, so `SameSite` stops nothing it posts there.
    #[tokio::test]
    async fn a_tenants_service_is_never_named_under_the_node_or_another_organisation() {
        use crate::domain::policies::tenant_names::{UNDER_ANOTHER, UNDER_THE_NODE};
        let (database, service) = a_tenants_service(Some("acme.com")).await;

        assert_eq!(
            named(&database, &service, "web.shop.node.example.com").await,
            UNDER_THE_NODE
        );
        assert_eq!(
            named(&database, &service, "login.globex.com").await,
            UNDER_ANOTHER
        );
        for theirs in ["web.shop.acme.com", "shop.example.org"] {
            let reason = named(&database, &service, theirs).await;
            assert!(
                reason != UNDER_THE_NODE && reason != UNDER_ANOTHER,
                "{theirs}: {reason}"
            );
        }
    }

    #[tokio::test]
    async fn on_a_node_that_asks_no_authority_a_wildcard_needs_no_zone() {
        let (database, service) = service().await;
        crate::infra::node::settings::set_acme_enabled(&database, false)
            .await
            .expect("acme off");

        let outcome = wildcard(&database, &service, false).await;

        assert!(
            !matches!(&outcome, Err(reason) if reason.contains("zone")),
            "{outcome:?}"
        );
    }
}
