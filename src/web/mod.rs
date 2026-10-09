//! The outer room: the router that mounts every feature's doors, and the
//! HTTP doors no single feature owns — the health check, the control
//! plane, the registry, and the page a secret is shown on once.

pub mod api;
pub mod console;
pub mod health_rest_controller;
pub mod registry_rest_controller;
pub mod remote_node_rest_controller;
