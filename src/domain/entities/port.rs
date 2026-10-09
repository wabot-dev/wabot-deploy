//! A port: what a service answers on, and the name it answers to.
//!
//! Three questions that used to be one column: what the process listens
//! on, whether that is reachable from outside the node, and whether it
//! answers HTTPS at a hostname. A port row answers all three, and the
//! common answer to the last two is "no".

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Port {
    pub id: String,
    pub service_id: String,
    /// What the process listens on inside the container.
    pub container_port: u16,
    /// The node's port when this one is published as raw TCP.
    pub host_port: Option<u16>,
    /// The hostname this port answers HTTPS on.
    pub hostname: Option<String>,
}
