//! The console's shell and the HTTP doors no single feature owns: the
//! router, the session, the layout, the shared pieces and words, the health
//! check, the control plane and the registry.

pub mod api;
pub mod console;
pub mod dtos;
pub mod health_rest_controller;
pub mod registry_rest_controller;
pub mod remote_node_rest_controller;
pub mod views;
