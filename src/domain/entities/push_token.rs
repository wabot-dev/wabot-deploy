//! A push token: what a `docker login` presents.

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushToken {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub created_at: i64,
    /// When something last authenticated with it — the answer to "is
    /// this one still in use" before somebody revokes it.
    pub last_used_at: Option<i64>,
}
