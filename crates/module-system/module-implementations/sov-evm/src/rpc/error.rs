//! Place where [`RlpConversionError`] is converted to [`EthApiError`]

use alloy_primitives::Bytes;
use reth_rpc_eth_types::{EthApiError, EthResult, RevertError, RpcInvalidTransactionError};
use revm::context::result::{ExecutionResult, HaltReason};

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
        ExecutionResult::Halt { reason, gas_used } => {
            Err(RpcInvalidTransactionError::halt(reason, gas_used).into())
        }
    }
}
