use wabot::sqlite::SqliteDatabase;

use crate::deploy::Deployer;
use crate::platform::projects::Project;
use crate::platform::services::{self, Service};
use crate::platform::{presets, quotas, replicas};

/// What a form asks for, before anybody has decided whose project it is.
#[derive(Clone, Copy)]
pub struct Asked<'a> {
    pub memory: &'a str,
    pub cpu: &'a str,
    pub disk: &'a str,
}

pub struct SetServiceResources<'a> {
    pub project: &'a Project,
    pub service: &'a Service,
    pub asked: Asked<'a>,
}

impl SetServiceResources<'_> {
    /// **The organisation's ceiling first, then the node's.** The tenant
    /// sum is local rows and can always be answered, it is the more
    /// general refusal — no node helps when there is no quota left — and
    /// a refusal should lead to the smallest correction.
    pub async fn run(
        &self,
        database: &SqliteDatabase,
        deployer: &Deployer,
    ) -> Result<Service, String> {
        // A managed engine's ceiling is a preset, because the number also
        // sets the engine's arithmetic; anything else takes a size.
        let memory = match self.service.kind.is_managed() {
            true => presets::parse(self.asked.memory),
            false => presets::parse_size(self.asked.memory),
        }?;
        let cpu = presets::parse_cores(self.asked.cpu)?;
        let disk = presets::parse_size(self.asked.disk)?;

        let copies = replicas::of_service(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .into_iter()
            .filter(|replica| !replica.evicted())
            .count()
            .max(1) as u64;

        if let Some(reason) = quotas::admits(
            database,
            self.project,
            self.service,
            memory,
            cpu,
            disk,
            copies,
        )
        .await
        .map_err(|error| error.to_string())?
        {
            return Err(reason);
        }
        if let Some(reason) = deployer.room_for(self.service, memory, cpu).await {
            return Err(reason);
        }

        for write in [
            services::set_memory_limit(database, &self.service.id, memory).await,
            services::set_cpu_limit(database, &self.service.id, cpu).await,
            services::set_disk_limit(database, &self.service.id, disk).await,
        ] {
            write.map_err(|error| error.to_string())?;
        }

        Ok(services::find(database, &self.service.id)
            .await
            .map_err(|error| error.to_string())?
            .unwrap_or_else(|| self.service.clone()))
    }
}
