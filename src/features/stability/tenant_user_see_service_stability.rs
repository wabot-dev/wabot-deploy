use wabot::sqlite::SqliteDatabase;

use crate::domain::authz::tenant_user;
use crate::domain::entities::account::Account;
use crate::features::stability::describe_service_stability_command::DescribeServiceStability;
use crate::features::stability::see_service_stability_request::SeeServiceStabilityRequest;
use crate::features::stability::service_stability_response::ServiceStabilityResponse;
use crate::infra::repositories::{access, services};

pub async fn run(
    database: &SqliteDatabase,
    account: &Account,
    request: &SeeServiceStabilityRequest,
) -> Result<ServiceStabilityResponse, String> {
    if !tenant_user::belongs_here(account) {
        return Err("that is not this console's to see".into());
    }
    let unreadable = |error: String| {
        tracing::warn!(%error, service = %request.service, "could not read a service");
        "the service could not be read".to_string()
    };
    let (project, _) = access::find_project(database, account, &request.project)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .ok_or_else(|| "no such project".to_string())?;
    let service = services::in_project(database, &project.id, &request.service)
        .await
        .map_err(|error| unreadable(error.to_string()))?
        .ok_or_else(|| "no such service".to_string())?;
    DescribeServiceStability {
        project: &project,
        service: &service,
        days: request.days,
        day: request.day,
        event: request.event.as_deref(),
    }
    .run(database)
    .await
}
