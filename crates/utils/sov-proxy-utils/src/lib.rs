//! Proxy utilities for querying node information from the database.
//!
//! This crate provides the [`Proxy`] struct to retrieve leader and follower
//! IP addresses from the PostgreSQL database atomically.

mod cluster_monitor;
mod file_writer;
mod node_discovery;
pub mod root_hash_checker;

pub use cluster_monitor::*;
pub use node_discovery::*;

#[cfg(test)]
mod tests;
