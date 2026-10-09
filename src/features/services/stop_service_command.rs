use crate::domain::entities::project::Project;
use crate::domain::entities::service::Service;
use crate::infra::deploy::Deployer;

/// **A stop has to travel.** Taking the copies down here and saying
/// nothing to the machines running the others left a service the console
/// called stopped still serving traffic elsewhere — the intent rides
/// beside the placement now.
pub struct StopService<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
}

impl StopService<'_> {
    pub async fn run(&self, deployer: &Deployer) -> Result<(), String> {
        deployer
            .stop(self.project, self.service)
            .await
            .map_err(|error| error.to_string())
    }
}
