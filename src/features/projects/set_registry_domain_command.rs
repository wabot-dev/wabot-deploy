use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::values::hostname::{Hostname, WILDCARD_REFUSED};
use crate::infra::deploy::dns::Resolution;
use crate::infra::deploy::Deployer;
use crate::infra::repositories::projects::{self, RegistryClaim};

pub struct SetRegistryDomain<'a> {
    pub project: &'a Project,
    pub domain: Option<&'a str>,
}

impl SetRegistryDomain<'_> {
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }

        let wanted = match self.domain.map(Hostname::parse) {
            Some(name) if !name.is_empty() => Some(name),
            _ => None,
        };

        if let Some(name) = &wanted {
            if name.is_wildcard() {
                return Err(WILDCARD_REFUSED.into());
            }
            refuse_what_answers_already(database, name.as_str()).await?;

            // A push and a far node's pull both reach this name, so it has
            // to arrive here — asked before saving, as the node's own
            // domain and an organisation's are, because a typo saved is a
            // `docker login` that cannot connect and a certificate ordered
            // for a name that will never validate.
            let node_domain = crate::infra::node::settings::domain(database).await;
            let reference = node_domain.as_deref().unwrap_or(name.as_str());
            let outcome =
                match crate::infra::deploy::dns::resolves_here(name.as_str(), reference).await {
                    Resolution::NoReference if node_domain.is_none() => Resolution::Missing,
                    outcome => outcome,
                };
            if !outcome.ok() {
                return Err(outcome.explain(name.as_str()));
            }
        }

        match projects::set_registry_domain(
            database,
            &self.project.id,
            wanted.as_ref().map(Hostname::as_str),
        )
        .await
        .map_err(|error| error.to_string())?
        {
            RegistryClaim::Set => {}
            RegistryClaim::HeldBy(holder) => {
                return Err(crate::shell::texts::sentences::name_is_taken(&holder));
            }
        }

        deployer.sync_routes().await;
        Ok(())
    }
}

/// A route is keyed by its host, so a registry name over one that already
/// answers would take it — the console, an organisation's, or a site.
async fn refuse_what_answers_already(database: &SqliteDatabase, name: &str) -> Result<(), String> {
    if crate::infra::node::settings::domain(database)
        .await
        .as_deref()
        == Some(name)
    {
        return Err(
            "that is this node's own name — a project's registry needs one of its own".into(),
        );
    }
    if let Ok(Some(tenant)) = crate::infra::repositories::tenants::by_domain(database, name).await {
        return Err(crate::shell::texts::sentences::name_is_taken(&tenant.name));
    }
    let ports = crate::infra::repositories::ports::all(database)
        .await
        .map_err(|error| error.to_string())?;
    let redirects = crate::infra::repositories::redirects::all(database)
        .await
        .map_err(|error| error.to_string())?;
    let served = ports
        .iter()
        .any(|port| port.hostname.as_deref() == Some(name))
        || redirects.iter().any(|redirect| redirect.hostname == name);
    if served {
        return Err(
            "a service answers on that name — a name is a site or a registry, not both".into(),
        );
    }
    Ok(())
}
