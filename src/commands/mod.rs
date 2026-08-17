//! One module per subcommand.
//!
//! Each takes an already-loaded [`Config`](crate::config::Config) and
//! returns an exit code, so `main` decides how the process ends and
//! the commands stay testable without one.

pub mod backup;
pub mod blobs;
pub mod containerd;
pub mod doctor;
pub mod install;
pub mod join;
pub mod passwd;
pub mod serve;
pub mod setup_token;
