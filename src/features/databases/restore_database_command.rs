use std::sync::Arc;

use wabot::sqlite::SqliteDatabase;

use crate::commands::backup::Restored;
use crate::config::Config;
use crate::entities::service::Service;

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

        crate::commands::backup::restore_one(
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
