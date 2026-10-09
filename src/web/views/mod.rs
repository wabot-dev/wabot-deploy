//! Everything the console draws.
//!
//! Four kinds, and the split is what stops a page becoming the only
//! place a piece of markup exists:
//!
//! - `pages` — one file per page, taking what it needs and returning
//!   markup. It does not query and it does not decide.
//! - `components` — the reusable pieces pages are built from.
//! - `layouts` — the document and the shell a page is rendered into.
//! - `texts` — what the console says, in both languages.

pub mod components;
pub mod layouts;
pub mod pages;
pub mod texts;
