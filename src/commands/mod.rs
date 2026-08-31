//! One module per subcommand.
//!
//! Each takes an already-loaded [`Config`](crate::config::Config) and
//! returns an exit code, so `main` decides how the process ends and
//! the commands stay testable without one.

pub mod add_project_member;
pub mod add_service_port;
pub mod backup;
pub mod blobs;
pub mod check_database_certificate;
pub mod choose_env_revision;
pub mod choose_release;
pub mod clean;
pub mod containerd;
pub mod create_database;
pub mod create_project;
pub mod create_project_token;
pub mod create_service;
pub mod delete_project;
pub mod delete_service;
pub mod delete_service_port;
pub mod destination;
pub mod dispatch_standbys;
pub mod doctor;
pub mod evict_service;
pub mod install;
pub mod join;
pub mod log_writer;
pub mod name_database;
pub mod passwd;
pub mod publish_database;
pub mod remove_project_member;
pub mod restore_database;
pub mod restore_database_into;
pub mod revoke_project_token;
pub mod s3;
pub mod serve;
pub mod set_service_edges;
pub mod set_service_env;
pub mod set_service_image;
pub mod set_service_logs;
pub mod set_service_placement;
pub mod set_service_resources;
pub mod set_service_tracking;
pub mod setup_token;
pub mod stop_service;
pub mod store_certificate_source;
