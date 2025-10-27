use alloy_eips::BlockId;
use alloy_rpc_types::error::EthRpcErrorCode;
use alloy_rpc_types::request::TransactionInputError;
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
    /// Thrown when historical data is not available because it has been pruned
    ///
    /// This error is intended for use as a standard response when historical data is
    /// requested that has been pruned according to the node's data retention policy.
    ///
    /// See also <https://eips.ethereum.org/EIPS/eip-4444>
    #[error("pruned history unavailable")]
    PrunedHistoryUnavailable,
    /// Thrown when an unknown block or transaction index is encountered
    #[error("unknown block or tx index")]
    UnknownBlockOrTxIndex,
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
    // /// Error related to signing
    // #[error(transparent)]
    // Signing(#[from] SignError),
    /// Some feature is unsupported
    #[error("unsupported")]
    Unsupported(&'static str),
    /// When the tracer config does not match the tracer
    #[error("invalid tracer config")]
    InvalidTracerConfig,
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
    // /// Error thrown when batch tx response channel fails
    // #[error(transparent)]
    // BatchTxRecvError(#[from] RecvError),
    /// Any other error
    #[error("{0}")]
    Other(Box<dyn ToRpcError>),
}

impl EthApiError {
    /// crates a new [`EthApiError::Other`] variant.
    pub fn other<E: ToRpcError>(err: E) -> Self {
        Self::Other(Box::new(err))
    }
}

impl From<EthApiError> for jsonrpsee_types::error::ErrorObject<'static> {
    fn from(error: EthApiError) -> Self {
        match error {
            EthApiError::FailedToDecodeSignedTransaction
            | EthApiError::InvalidTransactionSignature
            | EthApiError::EmptyRawTransactionData
            | EthApiError::ConflictingFeeFieldsInRequest
            | EthApiError::InvalidTracerConfig
            | EthApiError::TransactionConversionError => invalid_params_rpc_err(error.to_string()),
            EthApiError::InvalidTransaction(err) => err.into(),
            EthApiError::PrevrandaoNotSet
            | EthApiError::ExcessBlobGasNotSet
            | EthApiError::EvmCustom(_) => internal_rpc_err(error.to_string()),
            EthApiError::UnknownBlockOrTxIndex => {
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
            EthApiError::Unsupported(msg) => internal_rpc_err(msg),
            err @ EthApiError::TransactionInputError(_) => invalid_params_rpc_err(err.to_string()),
            EthApiError::PrunedHistoryUnavailable => rpc_error_with_code(4444, error.to_string()),
            EthApiError::Other(err) => err.to_rpc_error(),
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

impl From<Infallible> for EthApiError {
    fn from(_: Infallible) -> Self {
        unreachable!()
    }
}
