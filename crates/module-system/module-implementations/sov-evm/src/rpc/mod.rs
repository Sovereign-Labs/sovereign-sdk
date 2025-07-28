use std::convert::Infallible;

use alloy_primitives::TxKind;
use error::ensure_success;
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};
use reth_primitives::revm_primitives::{
    Address, AnalysisKind, BlockEnv, CfgEnv, CfgEnvWithHandlerCfg, EVMError, ExecutionResult,
    HaltReason, InvalidHeader, InvalidTransaction, TransactTo, TxEnv, B256, KECCAK_EMPTY, U256,
};
use reth_primitives::{TransactionSignedEcRecovered, U64};
use reth_rpc_eth_types::{EthApiError, RevertError, RpcInvalidTransactionError};
use reth_rpc_types::{ReceiptEnvelope, ReceiptWithBloom};
use revm::Database;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::{config_value, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, InfallibleStateAccessor, Spec};
use tracing::{debug, trace};

use crate::call::get_cfg_env_with_handler;
use crate::evm::db::EvmDb;
use crate::evm::executor;
use crate::evm::primitive_types::{Receipt, SealedBlock, TransactionSignedAndRecovered};
use crate::helpers::{
    from_primitive_with_hash, from_recovered_with_block_context, prepare_call_env,
};
use crate::{Evm, MIN_CREATE_GAS, MIN_TRANSACTION_GAS};

pub(crate) mod error;

#[rpc_gen(client, server)]
impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Handler for `net_version`
    #[rpc_method(name = "net_version")]
    pub fn net_version(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<String> {
        debug!("EVM module JSON-RPC request to `net_version`");

        // Network ID is the same as chain ID for most networks
        let chain_id = self
            .cfg
            .get(state)
            .unwrap_infallible()
            .expect("EVM config must be set at genesis")
            .chain_id;

        Ok(chain_id.to_string())
    }

    /// Handler for: `eth_chainId`
    #[rpc_method(name = "eth_chainId")]
    pub fn chain_id(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<Option<U64>> {
        let chain_id = self
            .cfg
            .get(state)
            .unwrap_infallible()
            .expect("EVM config must be set at genesis")
            .chain_id;
        debug!(
            chain_id = chain_id,
            "EVM module JSON-RPC request to `eth_chainId`"
        );
        Ok(Some(U64::from(chain_id)))
    }

    /// Handler for `eth_getBlockByHash`
    #[rpc_method(name = "eth_getBlockByHash")]
    pub fn get_block_by_hash(
        &self,
        block_hash: B256,
        details: Option<bool>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<reth_rpc_types::RichBlock>> {
        debug!(
            ?block_hash,
            "EVM module JSON-RPC request to `eth_getBlockByHash`"
        );

        let block_number_hex = self
            .block_hashes
            .get(&block_hash, state)
            .unwrap_infallible()
            .map(|number| hex::encode(number.to_be_bytes()));

        match block_number_hex {
            Some(block_number_hex) => {
                self.get_block_by_number(Some(block_number_hex), details, state)
            }
            None => Ok(None),
        }
    }

    /// Handler for: `eth_getBlockByNumber`
    #[rpc_method(name = "eth_getBlockByNumber")]
    pub fn get_block_by_number(
        &self,
        block_number: Option<String>,
        details: Option<bool>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<reth_rpc_types::RichBlock>> {
        debug!(
            block_number,
            "EVM module JSON-RPC request to `eth_getBlockByNumber`"
        );

        let block = self.get_sealed_block_by_number(block_number, state);

        // Build rpc header response
        let header = from_primitive_with_hash(block.header.clone());

        // Collect transactions with ids from db
        let transactions_with_ids = block.transactions.clone().map(|id| {
            let tx = self
                .transactions
                .get(id, state)
                .unwrap_infallible()
                .expect("Transaction must be set");
            (id, tx)
        });

        // Build rpc transactions response
        let transactions = match details {
            Some(true) => reth_rpc_types::BlockTransactions::Full(
                transactions_with_ids
                    .map(|(id, tx)| {
                        from_recovered_with_block_context(
                            tx.clone().into(),
                            block.header.hash(),
                            block.header.number,
                            block.header.base_fee_per_gas,
                            U256::from(id - block.transactions.start),
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => reth_rpc_types::BlockTransactions::Hashes({
                transactions_with_ids
                    .map(|(_, tx)| tx.signed_transaction.hash)
                    .collect::<Vec<_>>()
            }),
        };

        // Build rpc block response
        let block = reth_rpc_types::Block {
            header,
            transactions,
            uncles: Default::default(),
            size: Default::default(),
            withdrawals: Default::default(),
            other: Default::default(),
        };

        Ok(Some(block.into()))
    }

    /// Handler for: `eth_getBalance`
    #[rpc_method(name = "eth_getBalance")]
    pub fn get_balance(
        &self,
        address: Address,
        _block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U256> {
        // TODO: Implement block_number once we have archival state #951
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/951

        let balance = self
            .get_db(state)
            .basic(address)
            .unwrap_infallible()
            .map(|account| account.balance)
            .unwrap_or_default();

        debug!(
            %address,
            %balance,
            "EVM module JSON-RPC request to `eth_getBalance`"
        );

        Ok(balance)
    }

    /// Handler for: `eth_getStorageAt`
    #[rpc_method(name = "eth_getStorageAt")]
    pub fn get_storage_at(
        &self,
        address: Address,
        index: U256,
        _block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U256> {
        debug!("EVM module JSON-RPC request to `eth_getStorageAt`");

        // TODO: Implement block_number once we have archival state #951
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/951

        let storage_slot = self
            .account_storage
            .get(&(&address, &index), state)
            .unwrap_infallible()
            .unwrap_or_default();

        Ok(storage_slot)
    }

    /// Handler for: `eth_getTransactionCount`
    #[rpc_method(name = "eth_getTransactionCount")]
    pub fn get_transaction_count(
        &self,
        address: Address,
        _block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        // TODO: Implement block_number once we have archival state #882
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/882

        let nonce = self
            .get_db(state)
            .basic(address)
            .unwrap_infallible()
            .map(|account| account.nonce)
            .unwrap_or_default();

        debug!(%address, nonce, "EVM module JSON-RPC request to `eth_getTransactionCount`");

        Ok(U64::from(nonce))
    }

    /// Handler for: `eth_getCode`
    #[rpc_method(name = "eth_getCode")]
    pub fn get_code(
        &self,
        address: Address,
        _block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<reth_primitives::Bytes> {
        debug!("EVM module JSON-RPC request to `eth_getCode`");

        // TODO: Implement block_number once we have archival state #951
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/951

        let code = self
            .accounts
            .get(&address, state)
            .unwrap_infallible()
            .and_then(|account| {
                self.code
                    .get(&account.info.code_hash, state)
                    .unwrap_infallible()
            })
            .unwrap_or_default();

        Ok(code)
    }

    /// Handler for: `eth_feeHistory`
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "eth_feeHistory")]
    pub fn fee_history(&self) -> RpcResult<reth_rpc_types::FeeHistory> {
        debug!("EVM module JSON-RPC request to `eth_feeHistory`");

        Ok(reth_rpc_types::FeeHistory {
            base_fee_per_gas: Default::default(),
            gas_used_ratio: Default::default(),
            oldest_block: Default::default(),
            reward: Default::default(),
            blob_gas_used_ratio: Default::default(),
            // EIP-4844 related
            base_fee_per_blob_gas: Default::default(),
        })
    }

    /// Handler for: `eth_getTransactionByHash`
    #[rpc_method(name = "eth_getTransactionByHash")]
    pub fn get_transaction_by_hash(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<reth_rpc_types::Transaction>> {
        let tx_number = self
            .transaction_hashes
            .get(&hash, state)
            .unwrap_infallible();

        let transaction = tx_number.map(|number| {
            let tx = self
                .transactions
                .get(number, state)
                .unwrap_infallible()
                .unwrap_or_else(|| panic!("Transaction with known hash {} and number {} must be set in all {} transaction",
                                          hash,
                                          number,
                                          self.transactions.len(state).unwrap_infallible()
                ));

            let block = self
                .blocks
                .get(tx.block_number, state)
                .unwrap_infallible()
                .unwrap_or_else(|| panic!("Block with number {} for known transaction {} must be set",
                                          tx.block_number,
                                          tx.signed_transaction.hash));

            from_recovered_with_block_context(
                tx.into(),
                block.header.hash(),
                block.header.number,
                block.header.base_fee_per_gas,
                U256::from(tx_number.unwrap() - block.transactions.start),
            )
        });

        debug!(
            %hash,
            ?transaction,
            "EVM module JSON-RPC request to `eth_getTransactionByHash`"
        );

        Ok(transaction)
    }

    /// Handler for: `eth_getTransactionReceipt`
    #[rpc_method(name = "eth_getTransactionReceipt")]
    pub fn get_transaction_receipt(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Option<reth_rpc_types::TransactionReceipt>> {
        debug!(
            %hash,
            "EVM module JSON-RPC request to `eth_getTransactionReceipt`"
        );

        let tx_number = self
            .transaction_hashes
            .get(&hash, state)
            .unwrap_infallible();

        let receipt = tx_number.map(|number| {
            let tx = self
                .transactions
                .get(number, state)
                .unwrap_infallible()
                .expect("Transaction with known hash must be set");
            let block = self
                .blocks
                .get(tx.block_number, state)
                .unwrap_infallible()
                .expect("Block number for known transaction must be set");

            let receipt = self
                .receipts
                .get(tx_number.unwrap(), state)
                .unwrap_infallible()
                .expect("Receipt for known transaction must be set");

            build_rpc_receipt(block, tx, tx_number.unwrap(), receipt)
        });

        Ok(receipt)
    }

    /// Handler for: `eth_call`
    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    //https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc-types/src/eth/call.rs#L7
    #[rpc_method(name = "eth_call")]
    pub fn get_call(
        &self,
        request: reth_rpc_types::TransactionRequest,
        block_number: Option<String>,
        _state_overrides: Option<reth_rpc_types::state::StateOverride>,
        _block_overrides: Option<Box<reth_rpc_types::BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<reth_primitives::Bytes> {
        debug!("EVM module JSON-RPC request to `eth_call`");

        let block_env = match block_number {
            Some(ref block_number) if block_number == "pending" => self
                .block_env
                .get(state)
                .unwrap_infallible()
                .unwrap_or_default()
                .clone(),
            _ => {
                let block = self.get_sealed_block_by_number(block_number, state);
                BlockEnv::from(block)
            }
        };

        let tx_env = prepare_call_env(&block_env, request.clone()).unwrap();

        let cfg = self.cfg.get(state).unwrap_infallible().unwrap_or_default();
        let cfg_env = get_cfg_env_with_handler(&block_env, cfg, Some(get_cfg_env_template()));

        let evm_db: EvmDb<_, S> = self.get_db(state);

        let result = match executor::inspect(evm_db, &block_env, tx_env, cfg_env) {
            Ok(result) => result.result,
            Err(err) => return Err(eth_api_into_rpc_error(eth_from_infallible(err))),
        };

        ensure_success(result).map_err(eth_api_into_rpc_error)
    }

    /// Handler for: `eth_blockNumber`
    #[rpc_method(name = "eth_blockNumber")]
    pub fn block_number(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<U256> {
        let block_number = self.blocks.len(state).unwrap_infallible().saturating_sub(1);
        debug!(%block_number, "EVM module JSON-RPC request to `eth_blockNumber`");

        Ok(U256::from(block_number))
    }

    /// Handler for: `eth_estimateGas`
    // https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc/src/eth/api/call.rs#L172
    #[rpc_method(name = "eth_estimateGas")]
    pub fn eth_estimate_gas(
        &self,
        request: reth_rpc_types::TransactionRequest,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        debug!("EVM module JSON-RPC request to `eth_estimateGas`");
        let mut block_env = match block_number {
            Some(ref block_number) if block_number == "pending" => self
                .block_env
                .get(state)
                .unwrap_infallible()
                .unwrap_or_default()
                .clone(),
            _ => {
                let block = self.get_sealed_block_by_number(block_number, state);
                BlockEnv::from(block)
            }
        };

        let tx_env = prepare_call_env(&block_env, request.clone()).unwrap();
        trace!(?tx_env, "TxEnv is prepared");

        let cfg = self.cfg.get(state).unwrap_infallible().unwrap_or_default();
        let cfg_env_with_handler =
            get_cfg_env_with_handler(&block_env, cfg, Some(get_cfg_env_template()));

        let request_gas = request.gas;
        let request_gas_price = request.gas_price;
        let env_gas_limit = block_env.gas_limit;

        // get the highest possible gas limit, either the request's set value or the currently
        // configured gas limit
        let mut highest_gas_limit = request.gas.map(U256::from).unwrap_or(env_gas_limit);
        trace!(
            ?request_gas,
            ?request_gas_price,
            ?env_gas_limit,
            ?highest_gas_limit,
            "Gas limits"
        );

        let account = self
            .get_db(state)
            .basic(tx_env.caller)
            .unwrap_infallible()
            .unwrap_or_default();

        // if the request is a simple transfer, can we optimize?
        if tx_env.data.is_empty() {
            if let TransactTo::Call(to) = tx_env.transact_to {
                let to_account = self
                    .accounts
                    .get(&to, state)
                    .unwrap_infallible()
                    .map(|account| account.info)
                    .unwrap_or_default();
                if KECCAK_EMPTY == to_account.code_hash {
                    // simple transfer, check if the caller has sufficient funds
                    let available_funds = account.balance;

                    if tx_env.value > available_funds {
                        return Err(invalid_tx_into_rpc_error(
                            RpcInvalidTransactionError::InsufficientFundsForTransfer,
                        ));
                    }
                    return Ok(U64::from(MIN_TRANSACTION_GAS));
                }
            }
        }

        // check funds of the sender
        if tx_env.gas_price > U256::ZERO {
            // allowance is (balance - tx.value) / tx.gas_price
            let allowance = (account.balance - tx_env.value) / tx_env.gas_price;

            if highest_gas_limit > allowance {
                // cap the highest gas limit by max gas caller can afford with a given gas price
                highest_gas_limit = allowance;
            }
        }

        // if the provided gas limit is less than the computed cap, use that
        block_env.gas_limit = std::cmp::min(U256::from(tx_env.gas_limit), highest_gas_limit);
        trace!(?block_env, "Block env is configured");

        let evm_db = self.get_db(state);

        // execute the call without writing to db
        let result = executor::inspect(
            evm_db,
            &block_env,
            tx_env.clone(),
            cfg_env_with_handler.clone(),
        );

        // Exceptional case: init used too much gas, we need to increase the gas limit and try
        // again
        if let Err(EVMError::Transaction(InvalidTransaction::CallerGasLimitMoreThanBlock)) = result
        {
            // if price or limit was included in the request, then we can execute the request
            // again with the block's gas limit to check if revert is gas related or not
            if request_gas.is_some() || request_gas_price.is_some() {
                let evm_db = self.get_db(state);
                return Err(eth_api_into_rpc_error(map_out_of_gas_err(
                    block_env,
                    tx_env,
                    cfg_env_with_handler,
                    evm_db,
                )));
            }
        }

        let result = match result {
            Ok(result) => match result.result {
                ExecutionResult::Success { .. } => result.result,
                ExecutionResult::Halt { reason, gas_used } => {
                    return Err(invalid_tx_into_rpc_error(RpcInvalidTransactionError::halt(
                        reason, gas_used,
                    )))
                }
                ExecutionResult::Revert { output, .. } => {
                    // if price or limit was included in the request,
                    // then we can execute the request
                    // again with the block's gas limit to check if revert is gas related or not
                    return if request_gas.is_some() || request_gas_price.is_some() {
                        let evm_db = self.get_db(state);
                        Err(eth_api_into_rpc_error(map_out_of_gas_err(
                            block_env,
                            tx_env,
                            cfg_env_with_handler,
                            evm_db,
                        )))
                    } else {
                        // the transaction did revert
                        Err(invalid_tx_into_rpc_error(
                            RpcInvalidTransactionError::Revert(RevertError::new(output)),
                        ))
                    };
                }
            },
            Err(err) => return Err(eth_api_into_rpc_error(eth_from_infallible(err))),
        };

        // at this point, we know the call succeeded but want to find the _best_ (lowest) gas the
        // transaction succeeds with.
        // we find this by doing a binary search over the
        // possible range NOTE: this is the gas the transaction used, which is less than the
        // transaction requires succeeding
        let gas_used = result.gas_used();
        // the lowest value is capped by the gas it takes for a transfer
        let mut lowest_gas_limit = if tx_env.transact_to.is_create() {
            MIN_CREATE_GAS
        } else {
            MIN_TRANSACTION_GAS
        };
        let mut highest_gas_limit: u64 = highest_gas_limit.try_into().unwrap_or(u64::MAX);
        // pick a point that's close to the estimated gas
        let mut mid_gas_limit = std::cmp::min(
            gas_used * 3,
            ((highest_gas_limit as u128 + lowest_gas_limit as u128) / 2) as u64,
        );
        // binary search
        while (highest_gas_limit - lowest_gas_limit) > 1 {
            let mut tx_env = tx_env.clone();
            tx_env.gas_limit = mid_gas_limit;

            let evm_db = self.get_db(state);
            let result = executor::inspect(
                evm_db,
                &block_env,
                tx_env.clone(),
                cfg_env_with_handler.clone(),
            );

            // Exceptional case: init used too much gas, we need to increase the gas limit and try
            // again
            if let Err(EVMError::Transaction(InvalidTransaction::CallerGasLimitMoreThanBlock)) =
                result
            {
                // increase the lowest gas limit
                lowest_gas_limit = mid_gas_limit;

                // new midpoint
                mid_gas_limit = ((highest_gas_limit as u128 + lowest_gas_limit as u128) / 2) as u64;
                continue;
            }

            match result {
                Ok(result) => match result.result {
                    ExecutionResult::Success { .. } => {
                        // cap the highest gas limit with succeeding gas limit
                        highest_gas_limit = mid_gas_limit;
                    }
                    ExecutionResult::Revert { .. } => {
                        // increase the lowest gas limit
                        lowest_gas_limit = mid_gas_limit;
                    }
                    ExecutionResult::Halt { reason, .. } => {
                        match reason {
                            HaltReason::OutOfGas(_) => {
                                // increase the lowest gas limit
                                lowest_gas_limit = mid_gas_limit;
                            }
                            err => {
                                // these should be unreachable because we know the transaction succeeds,
                                // but we consider these cases an error
                                return Err(invalid_tx_into_rpc_error(
                                    RpcInvalidTransactionError::EvmHalt(err),
                                ));
                            }
                        }
                    }
                },
                Err(err) => {
                    return Err(eth_api_into_rpc_error(eth_from_infallible(err)));
                }
            };

            // new midpoint
            mid_gas_limit = ((highest_gas_limit as u128 + lowest_gas_limit as u128) / 2) as u64;
        }

        debug!(
            %highest_gas_limit,
            "EVM module JSON-RPC response from `eth_estimateGas`"
        );
        Ok(U64::from(highest_gas_limit))
    }
}

impl<S: Spec> Evm<S> {
    fn get_sealed_block_by_number(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> SealedBlock {
        // safe, finalized, and pending are not supported
        match block_number {
            Some(ref block_number) if block_number == "earliest" => self
                .blocks
                .get(0, state)
                .unwrap_infallible()
                .expect("Genesis block must be set"),
            Some(ref block_number) if block_number == "latest" => self
                .blocks
                .last(state)
                .unwrap_infallible()
                .expect("Head block must be set"),
            Some(ref block_number) => {
                // hex representation may have 0x prefix
                let block_number = u64::from_str_radix(block_number.trim_start_matches("0x"), 16)
                    .expect("Block number must be a valid hex number, with or without 0x prefix");
                self.blocks
                    .get(block_number, state)
                    .unwrap_infallible()
                    .expect("Block must be set")
            }
            None => self
                .blocks
                .last(state)
                .unwrap_infallible()
                .expect("Head block must be set"),
        }
    }
}

fn get_cfg_env_template() -> CfgEnv {
    let mut cfg_env = CfgEnv::default();
    // Reth sets this to true and uses only timeout, but other clients use this as a part of DOS attacks protection, with 100mln gas limit
    // https://github.com/paradigmxyz/reth/blob/62f39a5a151c5f4ddc9bf0851725923989df0412/crates/rpc/rpc/src/eth/revm_utils.rs#L215
    cfg_env.disable_block_gas_limit = false;
    cfg_env.disable_eip3607 = true;
    cfg_env.disable_base_fee = true;
    cfg_env.chain_id = config_value!("CHAIN_ID");
    cfg_env.perf_analyse_created_bytecodes = AnalysisKind::Analyse;
    cfg_env.limit_contract_code_size = None;
    cfg_env
}

// modified from: https://github.com/paradigmxyz/reth many times
pub(crate) fn build_rpc_receipt(
    block: SealedBlock,
    tx: TransactionSignedAndRecovered,
    tx_number: u64,
    receipt: Receipt,
) -> reth_rpc_types::TransactionReceipt {
    let transaction: TransactionSignedEcRecovered = tx.into();
    let from = transaction.signer();

    let block_hash = Some(block.header.hash());
    let block_number = Some(block.header.number);
    let transaction_hash = Some(transaction.hash);
    let transaction_index = tx_number - block.transactions.start;

    let logs: Vec<reth_rpc_types::Log> = receipt
        .receipt
        .logs
        .iter()
        .enumerate()
        .map(|(tx_log_idx, log)| reth_rpc_types::Log {
            inner: log.clone(),
            block_hash,
            block_number,
            block_timestamp: Some(block.header.timestamp),
            transaction_hash,
            transaction_index: Some(transaction_index),
            log_index: Some(receipt.log_index_start + tx_log_idx as u64),
            removed: false,
        })
        .collect();

    let logs_bloom = receipt.receipt.bloom_slow();

    let rpc_receipt = reth_rpc_types::Receipt {
        status: receipt.receipt.success.into(),
        cumulative_gas_used: receipt.receipt.cumulative_gas_used as u128,
        logs,
    };

    let (contract_address, to) = match transaction.transaction.kind() {
        TxKind::Create => (Some(from.create(transaction.transaction.nonce())), None),
        TxKind::Call(addr) => (None, Some(Address(*addr))),
    };

    reth_rpc_types::TransactionReceipt {
        inner: ReceiptEnvelope::Eip1559(ReceiptWithBloom::new(rpc_receipt, logs_bloom)),
        transaction_hash: transaction.hash,
        transaction_index: Some(transaction_index),
        block_hash,
        block_number,
        gas_used: receipt.gas_used as u128,
        effective_gas_price: transaction.effective_gas_price(block.header.base_fee_per_gas),
        blob_gas_used: None,
        blob_gas_price: None,
        from,
        to,
        contract_address,
        // TODO pre-byzantium receipts have a post-transaction state root
        state_root: None,
        authorization_list: transaction.authorization_list().map(|l| l.to_vec()),
    }
}

fn map_out_of_gas_err<Ws: InfallibleStateAccessor, S: Spec>(
    block_env: BlockEnv,
    mut tx_env: TxEnv,
    cfg_env_with_handler: CfgEnvWithHandlerCfg,
    db: EvmDb<Ws, S>,
) -> EthApiError
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    let req_gas_limit = tx_env.gas_limit;
    tx_env.gas_limit = block_env.gas_limit.to();
    let res = executor::inspect(db, &block_env, tx_env, cfg_env_with_handler).unwrap();
    match res.result {
        ExecutionResult::Success { .. } => {
            // a transaction succeeded by manually increasing the gas limit to
            // highest, which means the caller lacks funds to pay for the tx
            RpcInvalidTransactionError::BasicOutOfGas(req_gas_limit).into()
        }
        ExecutionResult::Revert { output, .. } => {
            // reverted again after bumping the limit
            RpcInvalidTransactionError::Revert(RevertError::new(output)).into()
        }
        ExecutionResult::Halt { reason, .. } => RpcInvalidTransactionError::EvmHalt(reason).into(),
    }
}

fn eth_from_infallible(err: EVMError<Infallible>) -> EthApiError {
    match err {
        EVMError::Transaction(err) => RpcInvalidTransactionError::from(err).into(),
        EVMError::Header(InvalidHeader::PrevrandaoNotSet) => EthApiError::PrevrandaoNotSet,
        EVMError::Header(InvalidHeader::ExcessBlobGasNotSet) => EthApiError::ExcessBlobGasNotSet,
        EVMError::Database(_) => {
            panic!("Infallible error triggered")
        }
        EVMError::Custom(data) => EthApiError::EvmCustom(data),
        EVMError::Precompile(data) => EthApiError::EvmPrecompile(data),
    }
}

/// Hack while reth is not upgraded for `jsonrpsee` 0.25
pub fn eth_api_into_rpc_error(eth_error: EthApiError) -> ErrorObjectOwned {
    ErrorObject::owned(500, format!("Eth Error: {eth_error:?}"), None::<()>)
}

/// Hack while reth is not upgraded for `jsonrpsee` 0.25
pub fn invalid_tx_into_rpc_error(rpc: RpcInvalidTransactionError) -> ErrorObjectOwned {
    eth_api_into_rpc_error(rpc.into())
}
