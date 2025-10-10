//! Place where [`RlpConversionError`] is converted to [`EthApiError`]

use std::error::Error;

use alloy_primitives::Bytes;
use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};
use revm::context::result::{EVMError, ExecutionResult, HaltReason, InvalidHeader};
use sov_modules_api::StateAccessor;
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
        ExecutionResult::Halt { reason, gas_used } => {
            Err(RpcInvalidTransactionError::halt(reason, gas_used).into())
        }
    }
}

pub fn eth_from_evm_error<Ws: StateAccessor>(err: EVMError<crate::db::Error<Ws>>) -> EthApiError {
    match err {
        EVMError::Transaction(err) => RpcInvalidTransactionError::from(err).into(),
        EVMError::Header(InvalidHeader::PrevrandaoNotSet) => EthApiError::PrevrandaoNotSet,
        EVMError::Header(InvalidHeader::ExcessBlobGasNotSet) => EthApiError::ExcessBlobGasNotSet,
        EVMError::Database(db_err) => db_err.into(),
        EVMError::Custom(data) => EthApiError::EvmCustom(data),
    }
}

impl<Ws: StateAccessor> From<crate::db::Error<Ws>> for EthApiError {
    fn from(err: crate::db::Error<Ws>) -> Self {
        EthApiError::EvmCustom(format!("Database error: {err}"))
    }
}

/// Hack while reth is not upgraded for `jsonrpsee` 0.25
pub fn eth_api_into_rpc_error(eth_error: EthApiError) -> ErrorObjectOwned {
    ErrorObject::owned(500, format!("Eth Error: {eth_error:?}"), None::<()>)
}

/// Converts internal error into rpc error
pub fn into_rpc_error(err: impl Error) -> ErrorObjectOwned {
    ErrorObject::owned(500, format!("{err}"), None::<()>)
}
