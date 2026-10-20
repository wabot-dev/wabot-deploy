use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ZoneCreatedResponse {
    pub name: String,
}
