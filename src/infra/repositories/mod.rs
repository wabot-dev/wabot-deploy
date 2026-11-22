//! One module per entity, wrapping its data access.
//!
//! Nothing here decides anything: the rules are on the entity and the
//! orchestration is in a command. See `docs/architecture-rules.md` §0.

pub mod access;
pub mod allowlist_sets;
pub mod allowlists;
pub mod config_history;
pub mod copy_metrics;
pub mod databases;
pub mod directories;
pub mod edges;
pub mod egress;
pub mod env_groups;
pub mod object_stores;
pub mod ports;
pub mod project_domain;
pub mod projects;
pub mod redirects;
pub mod registry_credentials;
pub mod releases;
pub mod replicas;
pub mod service_events;
pub mod services;
pub mod stand_ins;
pub mod tenants;
pub mod tokens;
pub mod volumes;
pub mod wal_archive;
pub mod zones;
