// https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc/src/eth/revm_utils.rs

use reth_primitives::{AccessList, H256, U256};
use reth_rpc_types::CallRequest;
use revm::primitives::{TransactTo, TxEnv};

use crate::error::rpc::{EthApiError, EthResult, RpcInvalidTransactionError};
use crate::primitive_types::BlockEnv;

/// Helper type for representing the fees of a [CallRequest]
pub(crate) struct CallFees {
    /// EIP-1559 priority fee
    max_priority_fee_per_gas: Option<U256>,
    /// Unified gas price setting
    ///
    /// Will be the configured `basefee` if unset in the request
    ///
    /// `gasPrice` for legacy,
    /// `maxFeePerGas` for EIP-1559
    gas_price: U256,
    /// Max Fee per Blob gas for EIP-4844 transactions
    // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
    #[allow(dead_code)]
    max_fee_per_blob_gas: Option<U256>,
}

// === impl CallFees ===

impl CallFees {
    /// Ensures the fields of a [CallRequest] are not conflicting.
    ///
    /// If no `gasPrice` or `maxFeePerGas` is set, then the `gas_price` in the returned `gas_price`
    /// will be `0`. See: <https://github.com/ethereum/go-ethereum/blob/2754b197c935ee63101cbbca2752338246384fec/internal/ethapi/transaction_args.go#L242-L255>
    ///
    /// # EIP-4844 transactions
    ///
    /// Blob transactions have an additional fee parameter `maxFeePerBlobGas`.
    /// If the `maxFeePerBlobGas` or `blobVersionedHashes` are set we treat it as an EIP-4844
    /// transaction.
    ///
    /// Note: Due to the `Default` impl of [BlockEnv] (Some(0)) this assumes the `block_blob_fee` is
    /// always `Some`
    fn ensure_fees(
        call_gas_price: Option<U256>,
        call_max_fee: Option<U256>,
        call_priority_fee: Option<U256>,
        block_base_fee: U256,
        blob_versioned_hashes: Option<&[H256]>,
        max_fee_per_blob_gas: Option<U256>,
        block_blob_fee: Option<U256>,
    ) -> EthResult<CallFees> {
        /// Ensures that the transaction's max fee is lower than the priority fee, if any.
        fn ensure_valid_fee_cap(
            max_fee: U256,
            max_priority_fee_per_gas: Option<U256>,
        ) -> EthResult<()> {
            if let Some(max_priority) = max_priority_fee_per_gas {
                if max_priority > max_fee {
                    // Fail early
                    return Err(
                        // `max_priority_fee_per_gas` is greater than the `max_fee_per_gas`
                        RpcInvalidTransactionError::TipAboveFeeCap.into(),
                    );
                }
            }
            Ok(())
        }

        let has_blob_hashes = blob_versioned_hashes
            .as_ref()
            .map(|blobs| !blobs.is_empty())
            .unwrap_or(false);

        match (
            call_gas_price,
            call_max_fee,
            call_priority_fee,
            max_fee_per_blob_gas,
        ) {
            (gas_price, None, None, None) => {
                // either legacy transaction or no fee fields are specified
                // when no fields are specified, set gas price to zero
                let gas_price = gas_price.unwrap_or(U256::ZERO);
                Ok(CallFees {
                    gas_price,
                    max_priority_fee_per_gas: None,
                    max_fee_per_blob_gas: has_blob_hashes.then_some(block_blob_fee).flatten(),
                })
            }
            (None, max_fee_per_gas, max_priority_fee_per_gas, None) => {
                // request for eip-1559 transaction
                let max_fee = max_fee_per_gas.unwrap_or(block_base_fee);
                ensure_valid_fee_cap(max_fee, max_priority_fee_per_gas)?;

                let max_fee_per_blob_gas = has_blob_hashes.then_some(block_blob_fee).flatten();

                Ok(CallFees {
                    gas_price: max_fee,
                    max_priority_fee_per_gas,
                    max_fee_per_blob_gas,
                })
            }
            (None, max_fee_per_gas, max_priority_fee_per_gas, Some(max_fee_per_blob_gas)) => {
                // request for eip-4844 transaction
                let max_fee = max_fee_per_gas.unwrap_or(block_base_fee);
                ensure_valid_fee_cap(max_fee, max_priority_fee_per_gas)?;

                // Ensure blob_hashes are present
                if !has_blob_hashes {
                    // Blob transaction but no blob hashes
                    return Err(RpcInvalidTransactionError::BlobTransactionMissingBlobHashes.into());
                }

                Ok(CallFees {
                    gas_price: max_fee,
                    max_priority_fee_per_gas,
                    max_fee_per_blob_gas: Some(max_fee_per_blob_gas),
                })
            }
            _ => {
                // this fallback covers incompatible combinations of fields
                Err(EthApiError::ConflictingFeeFieldsInRequest)
            }
        }
    }
}

// https://github.com/paradigmxyz/reth/blob/d8677b4146f77c7c82d659c59b79b38caca78778/crates/rpc/rpc/src/eth/revm_utils.rs#L201
pub(crate) fn prepare_call_env(block_env: &BlockEnv, request: CallRequest) -> EthResult<TxEnv> {
    let CallRequest {
        from,
        to,
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        gas,
        value,
        input,
        nonce,
        access_list,
        chain_id,
        ..
    } = request;

    let CallFees {
        max_priority_fee_per_gas,
        gas_price,
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
        max_fee_per_blob_gas: _,
    } = CallFees::ensure_fees(
        gas_price,
        max_fee_per_gas,
        max_priority_fee_per_gas,
        U256::from(block_env.basefee),
        // EIP-4844 related fields
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
        None,
        None,
        None,
    )?;

    let gas_limit = gas.unwrap_or(U256::from(block_env.gas_limit.min(u64::MAX)));

    let env = TxEnv {
        gas_limit: gas_limit
            .try_into()
            .map_err(|_| RpcInvalidTransactionError::GasUintOverflow)?,
        nonce: nonce
            .map(|n| {
                n.try_into()
                    .map_err(|_| RpcInvalidTransactionError::NonceTooHigh)
            })
            .transpose()?,
        caller: from.unwrap_or_default(),
        gas_price,
        gas_priority_fee: max_priority_fee_per_gas,
        transact_to: to.map(TransactTo::Call).unwrap_or_else(TransactTo::create),
        value: value.unwrap_or_default(),
        data: input
            .try_into_unique_input()?
            .map(|data| data.0)
            .unwrap_or_default(),
        chain_id: chain_id.map(|c| c.as_u64()),
        access_list: access_list.map(AccessList::flattened).unwrap_or_default(),
        // EIP-4844 related fields
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
        blob_hashes: vec![],
        max_fee_per_blob_gas: None,
    };

    Ok(env)
}
