//! Small shared things that carry their own validation.
//!
//! A value object is **immutable and validated at construction**: once
//! you hold one, it is right, and nothing downstream re-checks it. That
//! is what separates it from an entity — an entity has an identity and a
//! life, a value object is just a value that could not be wrong.

pub mod image_reference;
pub mod mount_point;
