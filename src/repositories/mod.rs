//! One module per entity, wrapping its data access.
//!
//! Nothing here decides anything: the rules are on the entity and the
//! orchestration is in a command. See `docs/architecture-rules.md` §0.

pub mod config_history;
pub mod edges;
pub mod ports;
pub mod projects;
pub mod releases;
pub mod replicas;
pub mod services;
pub mod tenants;
pub mod tokens;
pub mod volumes;
