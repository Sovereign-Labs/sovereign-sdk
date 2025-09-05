// Much of this code was copy-pasted from reth-evm, and we'd rather keep it as
// similar as possible to upstream than clean it up.
#![allow(clippy::match_same_arms)]

pub(crate) mod conversions;
/// EVM execution utilities
pub mod executor;
pub(crate) mod primitive_types;
pub use primitive_types::RlpEvmTransaction;
