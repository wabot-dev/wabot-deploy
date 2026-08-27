//! Pure rules that belong to no single entity.
//!
//! A table, a ladder, an admission sum. No storage, no actor, no
//! identity — which is what makes these the easiest things here to test.
//! See `docs/architecture-rules.md` §0.

pub mod access;
pub mod postgres;
pub mod presets;
pub mod project_domain;
pub mod quotas;
pub mod upstreams;
pub mod wal;
