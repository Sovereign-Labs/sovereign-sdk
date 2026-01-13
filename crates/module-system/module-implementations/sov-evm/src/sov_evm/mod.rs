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
pub use precompiles::{
    is_known_precompile, PrecompileError, PrecompileOutput, PrecompileResult, SovPrecompiles,
    IDENTITY_PRECOMPILE_ADDRESS,
};
