use wabot::sqlite::SqliteDatabase;

use crate::domain::entities::project::Project;
use crate::infra::deploy::Deployer;
use crate::infra::network::errand::EGRESS_SINCE;
use crate::infra::repositories::egress;

/// The nodes a project leaves for the internet through, on every node
/// running one of its copies and every exit.
///
/// Only a node that lets this one name it may be chosen, and none while a
/// node too old to carry a copy's connections to an exit holds one.
pub struct SetExits<'a> {
    pub project: &'a Project,
    pub exits: &'a [String],
}

impl SetExits<'_> {
    pub async fn run(&self, database: &SqliteDatabase, deployer: &Deployer) -> Result<(), String> {
        if !self.project.is_ours() {
            return Err("this project is administered from the node that sent it".into());
        }
        let candidates = deployer
            .exit_candidates()
            .await
            .map_err(|error| error.to_string())?;
        let mut chosen = Vec::new();
        for node_id in self.exits {
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.node_id == *node_id)
                .ok_or_else(|| {
                    format!("{node_id} does not carry this node's projects to the internet")
                })?;
            chosen.push(candidate.address);
        }
        if !chosen.is_empty() {
            let too_old = deployer
                .too_old_for_egress(self.project)
                .await
                .map_err(|error| error.to_string())?;
            if !too_old.is_empty() {
                return Err(format!(
                    "{} runs copies of this project on a release older than {EGRESS_SINCE}, \
                     which would send them out through itself. Update it first.",
                    too_old.join(", ")
                ));
            }
        }
        egress::set_exits(database, &self.project.id, &chosen)
            .await
            .map_err(|error| error.to_string())?;
        deployer.keep_projects_apart(false).await;
        if let Err(error) = deployer.tell_directory(self.project).await {
            tracing::warn!(project = %self.project.slug, %error, "could not tell the nodes which exits this project uses; the next pass will");
        }
        Ok(())
    }
}
