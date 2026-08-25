//! One file per page. See `docs/architecture-rules.md` §5.
//!
//! A page takes what it needs and returns markup. It does not query and
//! it does not decide.

pub mod guest_join_page;
pub mod tenant_user_people_page;
