//! What a use case needs to be asked.
//!
//! A controller builds one out of a form and hands it over. The use case
//! reads nothing else — no `HashMap`, no request, no session — which is
//! what makes it callable from a test, from the CLI and from an errand.
//!
//! **Not every use case has one.** A request earns its place when the
//! ask has more than one part, or when it comes off a form: those are
//! the two cases where a shape can be got wrong silently. A use case
//! given one identifier the router already parsed takes it as an
//! argument — a struct around a single `&str` is a name to keep
//! agreeing with and buys no check that the type did not already make.
//!
//! **A rule with a better sentence stays where the sentence is.** The
//! `Validate` derive answers "a field arrived, and it is the right
//! length"; it cannot say *no release chosen* or *that project belongs
//! to an organisation*. So a check here covers shape, and anything the
//! person can act on stays in the use case with its own words. A
//! constraint written in both places is a constraint that will disagree
//! with itself.

pub mod add_service_port;
pub mod create_database;
pub mod create_service;
pub mod invite_person;
pub mod invite_person_into_organisation;
pub mod join;
pub mod mint_join_token;
pub mod set_backup_plan;
pub mod set_bucket_credentials;
pub mod set_capabilities;
pub mod set_grants;
pub mod set_service_resources;
pub mod spend_join_token;
