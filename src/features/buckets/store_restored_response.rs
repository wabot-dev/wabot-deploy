use crate::domain::entities::service::Service;

/// The copy, made and not yet started, and the backup it will be brought
/// from at its first deployment.
pub struct StoreRestoredResponse {
    pub service: Service,
    pub from: String,
    pub taken_at: i64,
}
