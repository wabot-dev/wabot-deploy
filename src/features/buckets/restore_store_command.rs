use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::domain::policies::{garage, presets};
use crate::domain::values::allowlist::Door;
use crate::features::buckets::create_bucket_command::NOT_A_STORE;
use crate::features::buckets::create_store_command::CreateStore;
use crate::features::buckets::create_store_request::CreateStoreRequest;
use crate::features::buckets::restore_store_request::RestoreStoreRequest;
use crate::features::buckets::store_restored_response::StoreRestoredResponse;
use crate::infra::backup::Wanted;
use crate::infra::config::Config;
use crate::infra::repositories::{allowlists, object_stores, services};

/// **Into a copy, never over the original**, as a database restores. The
/// copy is a new store with the original's ceilings and allowlist and a name
/// of its own; its volume arrives at its first deployment, and its buckets
/// become rows once Garage is up on it.
pub struct RestoreStore<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub asked: &'a RestoreStoreRequest,
}

impl RestoreStore<'_> {
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        config: &Config,
    ) -> Result<StoreRestoredResponse, String> {
        if !self.service.kind.is_store() || self.service.project_id != self.project.id {
            return Err(NOT_A_STORE.into());
        }
        let at = match self.asked.at.trim() {
            "" => crate::infra::clock::now_ms(),
            typed => crate::infra::backup::parse_target(typed)
                .ok_or_else(|| format!("{typed:?} is not a moment. Try 2026-08-16 14:32"))?,
        };
        let container = format!("{}.{}", self.project.slug, self.service.slug);
        let (from, taken_at) = crate::infra::backup::find_copy(
            config,
            database,
            Wanted {
                container: &container,
                service: &self.service.id,
            },
            garage::DATA_VOLUME,
            at,
            None,
        )
        .await
        .map_err(|refusal| refusal.to_string())?;

        let disk = self
            .service
            .disk_limit
            .ok_or("the store has no disk ceiling to give its copy")?;
        let allowlist = allowlists::of(database, &Door::Service(self.service.id.clone()))
            .await
            .map_err(|error| error.to_string())?;
        let name = match self.asked.into.trim() {
            "" => format!("{}-restored", self.service.slug),
            typed => typed.to_string(),
        };
        let copy = CreateStore {
            project: self.project,
            asked: &CreateStoreRequest {
                project: self.project.slug.clone(),
                name,
                hostname: self.asked.hostname.clone(),
                memory: self
                    .service
                    .memory_limit
                    .unwrap_or(crate::features::buckets::create_store_command::DEFAULT_PRESET)
                    .to_string(),
                disk: presets::size_field(disk),
                allowlist,
            },
        }
        .run(database)
        .await?;

        if let Err(error) =
            object_stores::set_restored_from(database, &copy.id, Some(&from.display())).await
        {
            let _ = services::delete(database, &copy.id).await;
            return Err(error.to_string());
        }
        Ok(StoreRestoredResponse {
            service: copy,
            from: from.display(),
            taken_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_store_no_backup_holds_is_refused_and_nothing_is_made() {
        let (database, service) =
            crate::features::fixtures::store_of_buckets(1024 * 1024 * 1024).await;
        let project = crate::infra::repositories::projects::find(&database, &service.project_id)
            .await
            .expect("read")
            .expect("project");
        let dir = tempfile::tempdir().expect("dir");
        let mut config = Config::default();
        config.node.data_dir = dir.path().to_path_buf();

        let refused = RestoreStore {
            project: &project,
            service: &service,
            asked: &RestoreStoreRequest::default(),
        }
        .run(&database, &config)
        .await;

        assert!(refused.is_err());
        assert_eq!(
            services::all(&database, None)
                .await
                .expect("services")
                .len(),
            1,
            "no copy was made"
        );
    }

    #[tokio::test]
    async fn a_moment_nobody_could_have_meant_is_refused_with_how_to_write_one() {
        let (database, service) =
            crate::features::fixtures::store_of_buckets(1024 * 1024 * 1024).await;
        let project = crate::infra::repositories::projects::find(&database, &service.project_id)
            .await
            .expect("read")
            .expect("project");
        let refused = RestoreStore {
            project: &project,
            service: &service,
            asked: &RestoreStoreRequest {
                at: "last tuesday".into(),
                ..RestoreStoreRequest::default()
            },
        }
        .run(&database, &Config::default())
        .await;
        assert_eq!(
            refused.err().as_deref(),
            Some("\"last tuesday\" is not a moment. Try 2026-08-16 14:32")
        );
    }
}
