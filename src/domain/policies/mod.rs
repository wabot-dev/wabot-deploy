//! Pure rules that belong to no single entity.
//!
//! A table, a ladder, an arithmetic. No storage, no actor, no
//! identity — which is what makes these the easiest things here to test.
//! See `docs/architecture-rules.md` §0.

pub mod mysql;
pub mod postgres;
pub mod presets;
pub mod restarts;
pub mod wal;
pub mod zone_serial;
