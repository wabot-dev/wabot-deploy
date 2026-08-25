//! One file per use case, prefixed `node_user_`, `tenant_user_`,
//! `cli_user_` or `remote_node_`. See `docs/architecture-rules.md` §1.
//!
//! Flat rather than nested: a filename is what a stack trace prints, and
//! a directory carrying the actor would leave the file called
//! `create_project.rs`.

#[cfg(test)]
pub mod fixtures;

pub mod tenant_user_invite_person;
pub mod tenant_user_remove_person;
pub mod tenant_user_revoke_invitation;
pub mod tenant_user_set_person_role;
