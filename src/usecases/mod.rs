//! The use cases whose feature has no folder under `src/features` yet,
//! and the fixtures every use case's tests share. See
//! `docs/architecture-rules.md` §1 and §11.

#[cfg(test)]
pub mod fixtures;

pub mod node_user_add_service_port;
pub mod node_user_ask_node_once;
pub mod node_user_back_up_now;
pub mod node_user_create_service;
pub mod node_user_delete_service;
pub mod node_user_delete_service_port;
pub mod node_user_deploy_release;
pub mod node_user_drain_node;
pub mod node_user_evict_service;
pub mod node_user_forget_bucket_credentials;
pub mod node_user_forget_node;
pub mod node_user_mint_join_token;
pub mod node_user_revert_service_env;
pub mod node_user_revoke_authority;
pub mod node_user_set_backup_plan;
pub mod node_user_set_bucket_credentials;
pub mod node_user_set_capabilities;
pub mod node_user_set_certificate_source;
pub mod node_user_set_grants;
pub mod node_user_set_node_domain;
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
pub mod tenant_user_add_service_port;
pub mod tenant_user_create_service;
pub mod tenant_user_delete_service;
pub mod tenant_user_delete_service_port;
pub mod tenant_user_deploy_release;
pub mod tenant_user_evict_service;
pub mod tenant_user_revert_service_env;
pub mod tenant_user_set_service_edges;
pub mod tenant_user_set_service_env;
pub mod tenant_user_set_service_image;
pub mod tenant_user_set_service_logs;
pub mod tenant_user_set_service_placement;
pub mod tenant_user_set_service_resources;
pub mod tenant_user_set_service_tracking;
pub mod tenant_user_stop_service;
