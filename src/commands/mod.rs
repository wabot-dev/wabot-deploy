//! One module per subcommand.
//!
//! Each takes an already-loaded [`Config`](crate::config::Config) and
//! returns an exit code, so `main` decides how the process ends and
//! the commands stay testable without one.

pub mod containerd;
pub mod doctor;
pub mod install;
pub mod serve;
