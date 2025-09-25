use alloy_eips::BlockId;
use alloy_primitives::{Address, B256};
use alloy_rpc_types::error::EthRpcErrorCode;
use alloy_rpc_types::request::TransactionInputError;
use core::time::Duration;
use reth_errors::RethError;
use reth_primitives_traits::transaction::signed::RecoveryError;
use revm::context::result::InvalidTransaction;
use revm::context_interface::result::{EVMError, InvalidHeader};
use std::convert::Infallible;

use crate::utils::{
    block_id_to_str, internal_rpc_err, invalid_params_rpc_err, rpc_error_with_code,
};
use crate::{RpcInvalidTransactionError, ToRpcError};

/// Result alias
pub type EthResult<T> = Result<T, EthApiError>;

/// Errors that can occur when interacting with the `eth_` namespace
#[derive(Debug, thiserror::Error)]
pub enum EthApiError {
    /// When a raw transaction is empty
    #[error("empty transaction data")]
    EmptyRawTransactionData,
    /// When decoding a signed transaction fails
    #[error("failed to decode signed transaction")]
    FailedToDecodeSignedTransaction,
    /// When the transaction signature is invalid
    #[error("invalid transaction signature")]
    InvalidTransactionSignature,
    // /// Errors related to the transaction pool
    // #[error(transparent)]
    // PoolError(RpcPoolError),
    /// Header not found for block hash/number/tag
    #[error("header not found")]
    HeaderNotFound(BlockId),
    /// Header range not found for start block hash/number/tag to end block hash/number/tag
    #[error("header range not found, start block {0:?}, end block {1:?}")]
    HeaderRangeNotFound(BlockId, BlockId),
    /// Thrown when historical data is not available because it has been pruned
    ///
    /// This error is intended for use as a standard response when historical data is
    /// requested that has been pruned according to the node's data retention policy.
    ///
    /// See also <https://eips.ethereum.org/EIPS/eip-4444>
    #[error("pruned history unavailable")]
    PrunedHistoryUnavailable,
    /// Receipts not found for block hash/number/tag
    #[error("receipts not found")]
    ReceiptsNotFound(BlockId),
    /// Thrown when an unknown block or transaction index is encountered
    #[error("unknown block or tx index")]
    UnknownBlockOrTxIndex,
    /// When an invalid block range is provided
    #[error("invalid block range")]
    InvalidBlockRange,
    /// Thrown when the target block for proof computation exceeds the maximum configured window.
    #[error("distance to target block exceeds maximum proof window")]
    ExceedsMaxProofWindow,
    /// An internal error where prevrandao is not set in the evm's environment
    #[error("prevrandao not in the EVM's environment after merge")]
    PrevrandaoNotSet,
    /// `excess_blob_gas` is not set for Cancun and above
    #[error("excess blob gas missing in the EVM's environment after Cancun")]
    ExcessBlobGasNotSet,
    /// Thrown when a call or transaction request (`eth_call`, `eth_estimateGas`,
    /// `eth_sendTransaction`) contains conflicting fields (legacy, EIP-1559)
    #[error("both gasPrice and (maxFeePerGas or maxPriorityFeePerGas) specified")]
    ConflictingFeeFieldsInRequest,
    /// Errors related to invalid transactions
    #[error(transparent)]
    InvalidTransaction(#[from] RpcInvalidTransactionError),
    // /// Thrown when constructing an RPC block from primitive block data fails
    // #[error(transparent)]
    // InvalidBlockData(#[from] BlockError),
    /// Thrown when an `AccountOverride` contains conflicting `state` and `stateDiff` fields
    #[error("account {0:?} has both 'state' and 'stateDiff'")]
    BothStateAndStateDiffInOverride(Address),
    /// Other internal error
    #[error(transparent)]
    Internal(RethError),
    // /// Error related to signing
    // #[error(transparent)]
    // Signing(#[from] SignError),
    /// Thrown when a requested transaction is not found
    #[error("transaction not found")]
    TransactionNotFound,
    /// Some feature is unsupported
    #[error("unsupported")]
    Unsupported(&'static str),
    /// General purpose error for invalid params
    #[error("{0}")]
    InvalidParams(String),
    /// When the tracer config does not match the tracer
    #[error("invalid tracer config")]
    InvalidTracerConfig,
    /// When the percentile array is invalid
    #[error("invalid reward percentiles")]
    InvalidRewardPercentiles,
    /// Error thrown when a spawned blocking task failed to deliver an anticipated response.
    ///
    /// This only happens if the blocking task panics and is aborted before it can return a
    /// response back to the request handler.
    #[error("internal blocking task error")]
    InternalBlockingTaskError,
    /// Error thrown when a spawned blocking task failed to deliver an anticipated response
    #[error("internal eth error")]
    InternalEthError,
    /// Error thrown when a (tracing) call exceeds the configured timeout
    #[error("execution aborted (timeout = {0:?})")]
    ExecutionTimedOut(Duration),
    /// Internal Error thrown by the javascript tracer
    #[error("{0}")]
    InternalJsTracerError(String),
    #[error(transparent)]
    /// Call Input error when both `data` and `input` fields are set and not equal.
    TransactionInputError(#[from] TransactionInputError),
    /// Evm generic purpose error.
    #[error("Revm error: {0}")]
    EvmCustom(String),
    /// Bytecode override is invalid.
    ///
    /// This can happen if bytecode provided in an
    /// [`AccountOverride`](alloy_rpc_types_eth::state::AccountOverride) is malformed, e.g. invalid
    /// 7702 bytecode.
    // #[error("Invalid bytecode: {0}")]
    // InvalidBytecode(String),
    /// Error encountered when converting a transaction type
    #[error("Transaction conversion error")]
    TransactionConversionError,
    // /// Error thrown when tracing with a muxTracer fails
    // #[error(transparent)]
    // MuxTracerError(#[from] MuxError),
    /// Error thrown when waiting for transaction confirmation times out
    #[error(
        "Transaction {hash} was added to the mempool but wasn't confirmed within {duration:?}."
    )]
    TransactionConfirmationTimeout {
        /// Hash of the transaction that timed out
        hash: B256,
        /// Duration that was waited before timing out
        duration: Duration,
    },
    // /// Error thrown when batch tx response channel fails
    // #[error(transparent)]
    // BatchTxRecvError(#[from] RecvError),
    /// Error thrown when batch tx send channel fails
    #[error("Batch transaction sender channel closed")]
    BatchTxSendError,
    /// Any other error
    #[error("{0}")]
    Other(Box<dyn ToRpcError>),
}

impl EthApiError {
    /// crates a new [`EthApiError::Other`] variant.
    pub fn other<E: ToRpcError>(err: E) -> Self {
        Self::Other(Box::new(err))
    }

    /// Returns `true` if error is [`RpcInvalidTransactionError::GasTooHigh`]
    pub const fn is_gas_too_high(&self) -> bool {
        matches!(
            self,
            Self::InvalidTransaction(
                RpcInvalidTransactionError::GasTooHigh
                    | RpcInvalidTransactionError::GasLimitTooHigh
            )
        )
    }

    /// Returns `true` if error is [`RpcInvalidTransactionError::GasTooLow`]
    pub const fn is_gas_too_low(&self) -> bool {
        matches!(
            self,
            Self::InvalidTransaction(RpcInvalidTransactionError::GasTooLow)
        )
    }

    /// Returns the [`RpcInvalidTransactionError`] if this is a [`EthApiError::InvalidTransaction`]
    pub const fn as_invalid_transaction(&self) -> Option<&RpcInvalidTransactionError> {
        match self {
            Self::InvalidTransaction(e) => Some(e),
            _ => None,
        }
    }

    /// Converts this error into the rpc error object.
    pub fn into_rpc_err(self) -> jsonrpsee_types::error::ErrorObject<'static> {
        self.into()
    }
}

impl From<EthApiError> for jsonrpsee_types::error::ErrorObject<'static> {
    fn from(error: EthApiError) -> Self {
        match error {
            EthApiError::FailedToDecodeSignedTransaction
            | EthApiError::InvalidTransactionSignature
            | EthApiError::EmptyRawTransactionData
            | EthApiError::InvalidBlockRange
            | EthApiError::ExceedsMaxProofWindow
            | EthApiError::ConflictingFeeFieldsInRequest
            | EthApiError::BothStateAndStateDiffInOverride(_)
            | EthApiError::InvalidTracerConfig
            | EthApiError::TransactionConversionError
            | EthApiError::InvalidRewardPercentiles => invalid_params_rpc_err(error.to_string()),
            EthApiError::InvalidTransaction(err) => err.into(),
            EthApiError::PrevrandaoNotSet
            | EthApiError::ExcessBlobGasNotSet
            | EthApiError::Internal(_)
            | EthApiError::EvmCustom(_) => internal_rpc_err(error.to_string()),
            EthApiError::UnknownBlockOrTxIndex | EthApiError::TransactionNotFound => {
                rpc_error_with_code(EthRpcErrorCode::ResourceNotFound.code(), error.to_string())
            }
            // TODO(onbjerg): We rewrite the error message here because op-node does string matching
            // on the error message.
            //
            // Until https://github.com/ethereum-optimism/optimism/pull/11759 is released, this must be kept around.
            EthApiError::HeaderNotFound(id) => rpc_error_with_code(
                EthRpcErrorCode::ResourceNotFound.code(),
                format!("block not found: {}", block_id_to_str(id)),
            ),
            EthApiError::ReceiptsNotFound(id) => rpc_error_with_code(
                EthRpcErrorCode::ResourceNotFound.code(),
                format!("{error}: {}", block_id_to_str(id)),
            ),
            EthApiError::HeaderRangeNotFound(start_id, end_id) => rpc_error_with_code(
                EthRpcErrorCode::ResourceNotFound.code(),
                format!(
                    "{error}: start block: {}, end block: {}",
                    block_id_to_str(start_id),
                    block_id_to_str(end_id),
                ),
            ),
            err @ EthApiError::TransactionConfirmationTimeout { .. } => rpc_error_with_code(
                EthRpcErrorCode::TransactionConfirmationTimeout.code(),
                err.to_string(),
            ),
            EthApiError::Unsupported(msg) => internal_rpc_err(msg),
            EthApiError::InternalJsTracerError(msg) => internal_rpc_err(msg),
            EthApiError::InvalidParams(msg) => invalid_params_rpc_err(msg),
            err @ EthApiError::ExecutionTimedOut(_) => rpc_error_with_code(
                jsonrpsee_types::error::CALL_EXECUTION_FAILED_CODE,
                err.to_string(),
            ),
            err @ (EthApiError::InternalBlockingTaskError | EthApiError::InternalEthError) => {
                internal_rpc_err(err.to_string())
            }
            err @ EthApiError::TransactionInputError(_) => invalid_params_rpc_err(err.to_string()),
            EthApiError::PrunedHistoryUnavailable => rpc_error_with_code(4444, error.to_string()),
            EthApiError::Other(err) => err.to_rpc_error(),
            EthApiError::BatchTxSendError => {
                internal_rpc_err("Batch transaction sender channel closed".to_string())
            }
        }
    }
}

#[cfg(feature = "js-tracer")]
impl From<revm_inspectors::tracing::js::JsInspectorError> for EthApiError {
    fn from(error: revm_inspectors::tracing::js::JsInspectorError) -> Self {
        match error {
            err @ revm_inspectors::tracing::js::JsInspectorError::JsError(_) => {
                Self::InternalJsTracerError(err.to_string())
            }
            err => Self::InvalidParams(err.to_string()),
        }
    }
}

impl From<InvalidHeader> for EthApiError {
    fn from(value: InvalidHeader) -> Self {
        match value {
            InvalidHeader::ExcessBlobGasNotSet => Self::ExcessBlobGasNotSet,
            InvalidHeader::PrevrandaoNotSet => Self::PrevrandaoNotSet,
        }
    }
}

impl<T> From<EVMError<T, InvalidTransaction>> for EthApiError
where
    T: Into<Self>,
{
    fn from(err: EVMError<T, InvalidTransaction>) -> Self {
        match err {
            EVMError::Transaction(invalid_tx) => match invalid_tx {
                InvalidTransaction::NonceTooLow { tx, state } => {
                    Self::InvalidTransaction(RpcInvalidTransactionError::NonceTooLow { tx, state })
                }
                _ => RpcInvalidTransactionError::from(invalid_tx).into(),
            },
            EVMError::Header(err) => err.into(),
            EVMError::Database(err) => err.into(),
            EVMError::Custom(err) => Self::EvmCustom(err),
        }
    }
}

impl From<RecoveryError> for EthApiError {
    fn from(_: RecoveryError) -> Self {
        Self::InvalidTransactionSignature
    }
}

impl From<Infallible> for EthApiError {
    fn from(_: Infallible) -> Self {
        unreachable!()
    }
}
