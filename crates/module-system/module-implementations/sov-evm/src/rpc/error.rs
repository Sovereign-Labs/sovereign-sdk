//! Place where [`RlpConversionError`] is converted to [`EthApiError`]

use std::fmt::Display;

use alloy_primitives::Bytes;
use jsonrpsee::types::error::INTERNAL_ERROR_CODE;
use jsonrpsee::types::ErrorObjectOwned;
use revm::context::result::{ExecutionResult, HaltReason};
use sov_rpc_eth_types::{EthApiError, EthResult, RevertError, RpcInvalidTransactionError};

use crate::evm::conversions::RlpConversionError;

impl From<RlpConversionError> for EthApiError {
    fn from(value: RlpConversionError) -> Self {
        match value {
            RlpConversionError::EmptyRawTx => EthApiError::EmptyRawTransactionData,
            RlpConversionError::DeserializationFailed(_e) => {
                EthApiError::FailedToDecodeSignedTransaction
            }
            RlpConversionError::InvalidSignature => EthApiError::InvalidTransactionSignature,
        }
    }
}

/// Converts the evm [ExecutionResult] into a result
/// where [`Result::Ok`] variant is the output bytes if it if [`ExecutionResult::Success`].
pub(crate) fn ensure_success(result: ExecutionResult<HaltReason>) -> EthResult<Bytes> {
    match result {
        ExecutionResult::Success { output, .. } => Ok(output.into_data()),
        ExecutionResult::Revert { output, .. } => {
            Err(RpcInvalidTransactionError::Revert(RevertError::new(output)).into())
        }
        ExecutionResult::Halt { reason, gas, .. } => {
            Err(RpcInvalidTransactionError::halt(reason, gas.tx_gas_used()).into())
        }
    }
}

impl<E: std::fmt::Display> From<crate::db::Error<E>> for EthApiError {
    fn from(err: crate::db::Error<E>) -> Self {
        EthApiError::EvmCustom(format!("Database error: {err}"))
    }
}

/// Converts internal error into rpc error
pub fn into_rpc_error(err: impl Display) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(INTERNAL_ERROR_CODE, err.to_string(), None::<()>)
}
