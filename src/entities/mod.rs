//! The business models, with their rules and their validation.
//!
//! An entity decides what it may be. A rule about a `Service` lives on
//! `Service`, not in a free function beside the queries — see
//! `docs/architecture-rules.md` §0.

pub mod config_revision;
pub mod project;
pub mod push_token;
pub mod release;
pub mod service;
pub mod tenant;
