//! Custom EVM implementation that uses the custom handler and allows to override the EVM behavior.
//!
//! This module provides:
//! - `SovEvm`: Custom EVM wrapper with configurable gas charging behavior
//! - `SovPrecompiles`: Precompile provider supporting stateful sovereign precompiles
//! - `invoke_precompile`: Method for dispatching to precompile implementations with state access
mod api;
mod evm;
mod handler;
mod inspector;
mod precompiles;

pub use evm::SovEvm;
pub use inspector::StorageAccessInspector;
// Re-export precompile types for public API (some are not used internally)
#[allow(unused_imports)]
pub use precompiles::{
    is_known_precompile, PrecompileDb, PrecompileError, PrecompileOutput, PrecompileResult,
    SovPrecompiles, BANK_BALANCE_PRECOMPILE_ADDRESS, IDENTITY_PRECOMPILE_ADDRESS,
};
