//! Rpc server and client for `StorableMockDaService`.

mod client;
mod server;
mod types;

#[cfg(test)]
mod tests;

pub use client::{MockDaClientConfig, StorableMockDaClient};
pub use server::start_server;
