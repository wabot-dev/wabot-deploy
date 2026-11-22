//! The business models, with their rules and their validation.
//!
//! An entity decides what it may be. A rule about a `Service` lives on
//! `Service`, not in a free function beside the queries — see
//! `docs/architecture-rules.md` §0.

pub mod account;
pub mod allowlist_set;
pub mod bucket;
pub mod bucket_key;
pub mod config_revision;
pub mod copy_metric;
pub mod database;
pub mod dns_record;
pub mod dns_zone;
pub mod env_group;
pub mod object_store;
pub mod port;
pub mod project;
pub mod push_token;
pub mod redirect;
pub mod release;
pub mod replica;
pub mod service;
pub mod service_event;
pub mod tenant;
pub mod volume;
