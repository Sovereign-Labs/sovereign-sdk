//! Storage implementation based on [Nearly Optimal Merkle Tree(NOMT)](https://github.com/thrumdev/nomt/) implementation.

#[cfg(feature = "native")]
pub mod prover_storage;
pub mod zk_storage;
