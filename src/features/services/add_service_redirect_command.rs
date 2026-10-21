use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::domain::errors::PlatformError;
use crate::domain::values::hostname::Hostname;
use crate::features::services::add_service_redirect_request::AddServiceRedirectRequest;
use crate::infra::deploy::dns;
use crate::infra::repositories::{ports, redirects};

pub const NOT_ITS_NAME: &str = "a redirect goes to one of this service's own hostnames";
pub const A_DATABASE: &str = "a database answers no HTTPS, so there is nothing to redirect to";

pub struct AddServiceRedirect<'a> {
    pub service: &'a Service,
    pub asked: &'a AddServiceRedirectRequest,
}

impl AddServiceRedirect<'_> {
    /// The name is asked of DNS before it is stored, as a port's is: the
    /// edge answers for it with a certificate of its own, and an authority
    /// asked for a name that points elsewhere fails, against an account
    /// that locks at five failures an hour.
    pub async fn run(&self, database: &SqliteDatabase) -> Result<(), String> {
        if self.service.kind.is_managed() {
            return Err(A_DATABASE.into());
        }
        let to = Hostname::parse(&self.asked.to);
        let port = ports::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|port| port.hostname.as_deref() == Some(to.as_str()))
            .ok_or_else(|| NOT_ITS_NAME.to_string())?;

        let hostname = Hostname::parse(&self.asked.hostname);
        if hostname.is_wildcard() {
            return Err(crate::domain::values::hostname::WILDCARD_REFUSED.into());
        }
        let references = dns::references(database).await?;
        if references.is_empty() {
            return Err(
                "no node on this network answers for hostnames, so nothing could \
                        answer for this one"
                    .into(),
            );
        }
        let outcome = dns::resolves_to(hostname.as_str(), &references).await;
        if !outcome.ok() {
            return Err(outcome.explain(hostname.as_str()));
        }

        redirects::create(database, &port, hostname.as_str(), &outcome.nodes())
            .await
            .map(|_| ())
            .map_err(|error| match error {
                PlatformError::Refused(reason) => reason,
                other => {
                    tracing::warn!(error = %other, "could not save a redirect");
                    "that could not be saved".to_string()
                }
            })
    }
}
