//! One file per page. See `docs/architecture-rules.md` §5.
//!
//! A page takes what it needs and returns markup. It does not query and
//! it does not decide.

pub mod new_service_page;
pub mod revealed_page;
pub mod service_logs_page;
pub mod service_page;
pub mod service_settings_page;
