//! Everything that talks to something: the database, containerd, the
//! kernel, the network, GitHub, the disk, and the deploy loop that drives
//! them.

pub mod accounts;
pub mod backup;
pub mod bootstrap;
pub mod clock;
pub mod config;
pub mod db;
pub mod deploy;
pub mod edge;
pub mod ledger;
pub mod network;
pub mod node;
pub mod registry;
pub mod repositories;
pub mod runtime;
pub mod update;
