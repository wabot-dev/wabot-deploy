//! Small shared things that carry their own validation.
//!
//! A value object is **immutable and validated at construction**: once
//! you hold one, it is right, and nothing downstream re-checks it. That
//! is what separates it from an entity — an entity has an identity and a
//! life, a value object is just a value that could not be wrong.

pub mod account_event;
pub mod allowlist;
pub mod browser;
pub mod bucket_name;
pub mod capability;
pub mod command_line;
pub mod cors_origin;
pub mod destinations;
pub mod device_name;
pub mod hostname;
pub mod image_reference;
pub mod language;
pub mod mount_point;
pub mod name_server;
pub mod passkey_provider;
pub mod record_data;
pub mod record_name;
pub mod roles;
pub mod slug;
pub mod theme;
pub mod time_zone;
pub mod zone_name;
