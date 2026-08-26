//! One file per use case, prefixed `node_user_`, `tenant_user_`,
//! `cli_user_` or `remote_node_`. See `docs/architecture-rules.md` §1.
//!
//! Flat rather than nested: a filename is what a stack trace prints, and
//! a directory carrying the actor would leave the file called
//! `create_project.rs`.

#[cfg(test)]
pub mod fixtures;

pub mod guest_join_as_node_user;
pub mod guest_join_as_tenant_user;
pub mod node_user_add_project_member;
pub mod node_user_check_for_updates;
pub mod node_user_create_database;
pub mod node_user_create_organisation;
pub mod node_user_create_project;
pub mod node_user_create_project_token;
pub mod node_user_delete_organisation;
pub mod node_user_delete_project;
pub mod node_user_install_release;
pub mod node_user_invite_person;
pub mod node_user_invite_person_into_organisation;
pub mod node_user_name_database;
pub mod node_user_publish_database;
pub mod node_user_remove_person;
pub mod node_user_remove_project_member;
pub mod node_user_revoke_invitation;
pub mod node_user_revoke_project_token;
pub mod node_user_set_database_certificate;
pub mod node_user_set_node_domain;
pub mod node_user_set_organisation_domain;
pub mod node_user_set_organisation_quota;
pub mod node_user_set_person_role;
pub mod node_user_set_reserves;
pub mod tenant_user_add_project_member;
pub mod tenant_user_create_database;
pub mod tenant_user_create_project;
pub mod tenant_user_create_project_token;
pub mod tenant_user_delete_project;
pub mod tenant_user_invite_person;
pub mod tenant_user_name_database;
pub mod tenant_user_publish_database;
pub mod tenant_user_remove_person;
pub mod tenant_user_remove_project_member;
pub mod tenant_user_revoke_invitation;
pub mod tenant_user_revoke_project_token;
pub mod tenant_user_set_database_certificate;
pub mod tenant_user_set_person_role;
