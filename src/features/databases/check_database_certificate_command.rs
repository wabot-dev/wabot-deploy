use wabot::sqlite::SqliteDatabase;

use crate::config::Config;
use crate::edge::policy::RenewWith;
use crate::entities::project::Project;
use crate::entities::service::Service;

/// **ACME is refused unless both names resolve here**, and both is the
/// point: the read pool's name is derived from the primary's, so an
/// operator who added one DNS record and not the other would have an
/// order that cannot pass — twice a day, against an authority that locks
/// the account at five failures an hour.
///
/// The renewal loop makes the same check before it spends anything, so
/// this is not what protects the account. It is what gives an answer
/// *now*, in the form, instead of a silence somebody has to go and read
/// the certificate page to understand.
pub struct CheckDatabaseCertificate<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub renew_with: &'a RenewWith,
}

impl CheckDatabaseCertificate<'_> {
    pub async fn run(&self, database: &SqliteDatabase, config: &Config) -> Result<String, String> {
        let names =
            crate::deploy::certificate_names(database, config, self.project, self.service).await;
        let Some(name) = names.first().cloned() else {
            return Err("this database has no name yet".into());
        };

        if self.renew_with == &RenewWith::Acme {
            let Some(node_domain) = crate::node::settings::domain(database, config).await else {
                return Err(
                    "this node has no domain of its own, so it cannot check whether \
                            those names point here — set one on the node page first"
                        .into(),
                );
            };
            // Named individually when one fails: "it does not resolve"
            // about a name the operator never typed is a message they
            // cannot act on.
            for name in [name.clone(), crate::deploy::hosts::pool_name(&name)] {
                let outcome = crate::deploy::dns::resolves_here(&name, &node_domain).await;
                if !outcome.ok() {
                    return Err(outcome.explain(&name));
                }
            }
        }

        Ok(name)
    }
}
