//! The refusal-or-storage answer every repository, entity and command
//! gives.
//!
//! Two variants and the difference matters at the door: a **refusal** is
//! the caller's to fix and says so; a **storage** failure is ours and
//! does not describe itself over HTTP.

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("storage: {0}")]
    Storage(#[from] wabot::sqlite::SqliteError),
    #[error("{0}")]
    Refused(String),
}

pub type PlatformResult<T> = Result<T, PlatformError>;

/// See the note on `AccountError`'s conversion: a refusal is the
/// caller's to fix and says so; a storage failure is ours and does not
/// describe itself over HTTP.
impl From<PlatformError> for wabot::rest::RestError {
    fn from(error: PlatformError) -> Self {
        match error {
            PlatformError::Refused(message) => wabot::rest::RestError::Client {
                status: 400,
                message,
            },
            other => {
                tracing::error!(error = %other, "platform operation failed");
                wabot::rest::RestError::Internal("platform operation failed".into())
            }
        }
    }
}
