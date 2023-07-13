//! The `node` module defines types traits which are used by the full node but
//! are not part of the rollup's state machine. These types/traits are never invoked
//! inside of a zkVM, so they may be non-deterministic, have access to networking/disk, etc.
pub mod rpc;
pub mod services;
