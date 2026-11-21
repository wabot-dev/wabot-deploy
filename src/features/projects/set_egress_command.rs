use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::domain::values::destinations::Destinations;
use crate::infra::deploy::Deployer;
use crate::infra::network::errand::EGRESS_SINCE;
use crate::infra::repositories::egress;

/// Where a project's containers may connect past its own network, on
/// every node running one.
///
/// A node too old to enforce a list would run the project's copies with
/// none, so a list that is not anywhere waits until every one is new
/// enough.
pub struct SetEgress<'a> {
    pub project: &'a Project,
    pub destinations: &'a Destinations,
}

impl SetEgress<'_> {
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        if !self.destinations.is_anywhere() {
            let too_old = deployer
                .too_old_for_egress(self.project)
                .await
                .map_err(|error| error.to_string())?;
            if !too_old.is_empty() {
                return Err(format!(
                    "{} runs copies of this project on a release older than {EGRESS_SINCE}, \
                     which would let them reach anything. Update it first, or keep 0.0.0.0/0 \
                     on the list.",
                    too_old.join(", ")
                ));
            }
        }
        egress::set(database, &self.project.id, self.destinations)
            .await
            .map_err(|error| error.to_string())?;
        deployer.keep_projects_apart(false).await;
        if let Err(error) = deployer.tell_directory(self.project).await {
            tracing::warn!(project = %self.project.slug, %error, "could not tell the nodes where this project may connect; the next pass will");
        }
        Ok(())
    }
}
