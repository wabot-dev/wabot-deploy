use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::service::Service;
use crate::infra::backup::Restored;
use crate::infra::config::Config;

/// **Into a copy, never over the original.** What this asks for is one
/// database's volume out of one backup, unpacked beside the original,
/// which goes on serving — nothing is taken away, and comparing the two
/// is most of what a recovery is.
pub struct RestoreDatabase<'a> {
    pub service: &'a Service,
    /// The moment to reach. Empty means as far as the archived log goes.
    pub at: &'a str,
    /// What to call the copy. Empty takes the default name.
    pub into: &'a str,
}

impl RestoreDatabase<'_> {
    pub async fn run(
        &self,
        database: &Arc<SqliteDatabase>,
        config: &Config,
    ) -> Result<Restored, String> {
        if !self.service.kind.is_managed() {
            return Err("that service is not a managed database".into());
        }
        if let Some(node) =
            crate::infra::repositories::databases::primary_elsewhere(database, &self.service.id)
                .await
                .map_err(|error| error.to_string())?
        {
            return Err(format!(
                "it runs on {}, which takes its backups and archives its log — there is nothing \
                 of it on this machine to restore from",
                node.name
            ));
        }

        crate::infra::backup::restore_one(
            config,
            database,
            &self.service.slug,
            (!self.at.is_empty()).then_some(self.at),
            (!self.into.is_empty()).then_some(self.into),
            None,
        )
        .await
        .map_err(|refusal| refusal.to_string())
    }
}
