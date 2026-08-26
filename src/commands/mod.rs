//! One module per subcommand.
//!
//! Each takes an already-loaded [`Config`](crate::config::Config) and
//! returns an exit code, so `main` decides how the process ends and
//! the commands stay testable without one.

pub mod add_project_member;
pub mod backup;
pub mod blobs;
pub mod check_database_certificate;
pub mod choose_release;
pub mod clean;
pub mod containerd;
pub mod create_database;
pub mod create_project;
pub mod create_project_token;
pub mod create_service;
pub mod delete_project;
pub mod delete_service_port;
pub mod destination;
pub mod doctor;
pub mod install;
pub mod join;
pub mod log_writer;
pub mod name_database;
pub mod passwd;
pub mod publish_database;
pub mod remove_project_member;
pub mod revoke_project_token;
pub mod s3;
pub mod serve;
pub mod set_service_image;
pub mod set_service_logs;
pub mod set_service_tracking;
pub mod setup_token;
