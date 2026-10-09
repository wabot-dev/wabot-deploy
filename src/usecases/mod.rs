//! One file per use case, prefixed `node_user_`, `tenant_user_`,
//! `cli_user_` or `remote_node_`. See `docs/architecture-rules.md` §1.
//!
//! Flat rather than nested: a filename is what a stack trace prints, and
//! a directory carrying the actor would leave the file called
//! `create_project.rs`.

#[cfg(test)]
pub mod fixtures;

pub mod guest_claim_the_node;
pub mod guest_join_as_node_user;
pub mod guest_join_as_tenant_user;
pub mod guest_sign_in;
pub mod node_user_add_project_member;
pub mod node_user_add_service_port;
pub mod node_user_ask_node_once;
pub mod node_user_back_up_now;
pub mod node_user_check_for_updates;
pub mod node_user_create_organisation;
pub mod node_user_create_project;
pub mod node_user_create_project_token;
pub mod node_user_create_service;
pub mod node_user_delete_organisation;
pub mod node_user_delete_project;
pub mod node_user_delete_service;
pub mod node_user_delete_service_port;
pub mod node_user_deploy_release;
pub mod node_user_drain_node;
pub mod node_user_evict_service;
pub mod node_user_forget_bucket_credentials;
pub mod node_user_forget_node;
pub mod node_user_install_release;
pub mod node_user_invite_owner;
pub mod node_user_invite_person;
pub mod node_user_mint_join_token;
pub mod node_user_remove_owner;
pub mod node_user_remove_person;
pub mod node_user_remove_project_member;
pub mod node_user_revert_service_env;
pub mod node_user_revoke_authority;
pub mod node_user_revoke_invitation;
pub mod node_user_revoke_project_token;
pub mod node_user_set_backup_plan;
pub mod node_user_set_bucket_credentials;
pub mod node_user_set_capabilities;
pub mod node_user_set_certificate_source;
pub mod node_user_set_grants;
pub mod node_user_set_node_domain;
pub mod node_user_set_organisation_domain;
pub mod node_user_set_organisation_quota;
pub mod node_user_set_person_role;
pub mod node_user_set_reserves;
pub mod node_user_set_service_edges;
pub mod node_user_set_service_env;
pub mod node_user_set_service_image;
pub mod node_user_set_service_logs;
pub mod node_user_set_service_placement;
pub mod node_user_set_service_resources;
pub mod node_user_set_service_tracking;
pub mod node_user_set_wanted_version;
pub mod node_user_spend_join_token;
pub mod node_user_stop_service;
pub mod node_user_withdraw_enrolment;
pub mod tenant_user_add_project_member;
pub mod tenant_user_add_service_port;
pub mod tenant_user_create_project;
pub mod tenant_user_create_project_token;
pub mod tenant_user_create_service;
pub mod tenant_user_delete_project;
pub mod tenant_user_delete_service;
pub mod tenant_user_delete_service_port;
pub mod tenant_user_deploy_release;
pub mod tenant_user_evict_service;
pub mod tenant_user_invite_person;
pub mod tenant_user_remove_person;
pub mod tenant_user_remove_project_member;
pub mod tenant_user_revert_service_env;
pub mod tenant_user_revoke_invitation;
pub mod tenant_user_revoke_project_token;
pub mod tenant_user_set_person_role;
pub mod tenant_user_set_service_edges;
pub mod tenant_user_set_service_env;
pub mod tenant_user_set_service_image;
pub mod tenant_user_set_service_logs;
pub mod tenant_user_set_service_placement;
pub mod tenant_user_set_service_resources;
pub mod tenant_user_set_service_tracking;
pub mod tenant_user_stop_service;
