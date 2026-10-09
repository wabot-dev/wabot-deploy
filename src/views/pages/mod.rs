//! One file per page. See `docs/architecture-rules.md` §5.
//!
//! A page takes what it needs and returns markup. It does not query and
//! it does not decide.

pub mod guest_setup_page;
pub mod guest_sign_in_page;
pub mod new_service_page;
pub mod node_user_invite_node_page;
pub mod node_user_join_network_page;
pub mod node_user_network_page;
pub mod node_user_node_settings_page;
pub mod node_user_remote_node_page;
pub mod node_user_this_node_page;
pub mod revealed_page;
pub mod service_logs_page;
pub mod service_page;
pub mod service_settings_page;
