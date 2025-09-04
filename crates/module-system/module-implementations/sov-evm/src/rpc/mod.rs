use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_consensus::{Transaction as TransactionTrait, TxReceipt};
use alloy_primitives::{Address, U64};
use alloy_primitives::{Bytes, TxKind, B256, U256};
use alloy_rpc_types::{
    state::StateOverride, Block, BlockOverrides, BlockTransactions, FeeHistory, Log,
    ReceiptEnvelope, ReceiptWithBloom, Transaction, TransactionReceipt, TransactionRequest,
};
use error::ensure_success;
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};
use reth_primitives::{Recovered, TransactionSigned};
use reth_rpc_eth_types::{EthApiError, RevertError, RpcInvalidTransactionError};
use revm::context::result::{
    EVMError, ExecutionResult, HaltReason, InvalidHeader, InvalidTransaction,
};
use revm::context::{BlockEnv, CfgEnv, TransactTo, TxEnv};
use revm::Database;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::{config_value, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, InfallibleStateAccessor, Spec, StateAccessor};
use tracing::{debug, trace};

use crate::db::EvmDb;
use crate::evm::executor;
use crate::evm::primitive_types::{Receipt, SealedBlock, TransactionSignedAndRecovered};
use crate::executor::get_cfg_env;
use crate::helpers::{
    from_primitive_with_hash, from_recovered_with_block_context, prepare_call_env,
};
use crate::primitive_types::MaybeSealedBlock;
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
        let chain_id = self.cfg_infallible(state).chain_spec.chain_id;
        Ok(chain_id.to_string())
    }

    /// Handler for: `eth_chainId`
    #[rpc_method(name = "eth_chainId")]
    pub fn chain_id(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<Option<U64>> {
        let chain_id = self.cfg_infallible(state).chain_spec.chain_id;
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
    ) -> RpcResult<Option<Block>> {
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
    ) -> RpcResult<Option<Block>> {
        debug!(
            block_number,
            "EVM module JSON-RPC request to `eth_getBlockByNumber`"
        );

        let block = self.get_sealed_block_by_number(block_number, state);

        // Build rpc header response
        let header = from_primitive_with_hash(block.header.clone());

        // Collect transactions with ids from db
        let transactions_with_index = block.transactions.clone().map(|index| {
            let tx = self
                .transactions
                .get(&index, state)
                .unwrap_infallible()
                .expect("Transaction must be set");
            (index, tx)
        });

        // Build rpc transactions response
        let transactions = match details {
            Some(true) => BlockTransactions::Full(
                transactions_with_index
                    .map(|(index, tx)| {
                        from_recovered_with_block_context(
                            tx.clone().into(),
                            block.header.seal(),
                            block.header.number,
                            block.header.base_fee_per_gas,
                            U256::from(index - block.transactions.start),
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => BlockTransactions::Hashes({
                transactions_with_index
                    .map(|(_, tx)| *tx.signed_transaction.hash())
                    .collect::<Vec<_>>()
            }),
        };

        // Build rpc block response
        let block = Block {
            header,
            transactions,
            ..Default::default()
        };

        Ok(Some(block))
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
            .map_err(EthApiError::from)?
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

        let ethereum_address: EthereumAddress = address.into();
        let credential_id = ethereum_address.as_credential_id();

        let nonce = self
            .uniqueness_module
            .next_nonce(&credential_id, state)
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
    ) -> RpcResult<Bytes> {
        debug!("EVM module JSON-RPC request to `eth_getCode`");

        // TODO: Implement block_number once we have archival state #951
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/951

        let code = self
            .accounts
            .get(&address, state)
            .unwrap_infallible()
            .and_then(|account| self.code.get(&account.code_hash, state).unwrap_infallible())
            .unwrap_or_default();

        Ok(code)
    }

    /// Handler for: `eth_feeHistory`
    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/502
    #[rpc_method(name = "eth_feeHistory")]
    pub fn fee_history(&self) -> RpcResult<FeeHistory> {
        debug!("EVM module JSON-RPC request to `eth_feeHistory`");

        Ok(FeeHistory {
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
    ) -> RpcResult<Option<Transaction>> {
        let tx_number = self.get_tx_index_by_hash(&hash, state);

        let transaction = tx_number.map(|number| {
            let tx = self.transaction(number, state).unwrap();
            let block = self.block(tx.block_number, state);

            from_recovered_with_block_context(
                tx.into(),
                block.header.seal(),
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
    ) -> RpcResult<Option<TransactionReceipt>> {
        debug!(
            %hash,
            "EVM module JSON-RPC request to `eth_getTransactionReceipt`"
        );
        let Some(number) = self.get_tx_index_by_hash(&hash, state) else {
            return Ok(None);
        };

        let tx = self.transaction(number, state).unwrap();
        // The block may be `None` for a few seconds after the tx is processed
        let block = self.get_maybe_sealed_block(&tx, state);
        let receipt = self.receipt(number, state).unwrap();
        let receipt = build_rpc_receipt(block, tx, number, receipt);

        Ok(Some(receipt))
    }

    /// Handler for: `eth_call`
    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    //https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc-types/src/eth/call.rs#L7
    #[rpc_method(name = "eth_call")]
    pub fn get_call(
        &self,
        request: TransactionRequest,
        block_number: Option<String>,
        _state_overrides: Option<StateOverride>,
        _block_overrides: Option<Box<BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Bytes> {
        debug!("EVM module JSON-RPC request to `eth_call`");

        let block_env = self.resolve_block_env(block_number, state);
        let tx_env = prepare_call_env(&block_env, request.clone()).unwrap();

        let cfg = self.cfg_infallible(state);
        let cfg_env = get_cfg_env(&block_env, cfg, Some(get_cfg_env_template()));

        let evm_db: EvmDb<_, S> = self.get_db(state);

        let result = match executor::call(evm_db, &block_env, tx_env, cfg_env) {
            Ok(result) => result,
            Err(err) => return Err(eth_api_into_rpc_error(eth_from_evm_error(err))),
        };

        ensure_success(result).map_err(eth_api_into_rpc_error)
    }

    /// Handler for: `eth_blockNumber`
    #[rpc_method(name = "eth_blockNumber")]
    pub fn block_number(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<U256> {
        let block_number_range = self
            .block_numbers
            .get(state)
            .unwrap_infallible()
            .expect("Block number must be set");
        Ok(U256::from(*block_number_range.end()))
    }

    /// Handler for: `eth_estimateGas`
    // https://github.com/paradigmxyz/reth/blob/main/crates/rpc/rpc/src/eth/api/call.rs#L172
    #[rpc_method(name = "eth_estimateGas")]
    pub fn eth_estimate_gas(
        &self,
        request: TransactionRequest,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<U64> {
        debug!("EVM module JSON-RPC request to `eth_estimateGas`");
        let mut block_env = self.resolve_block_env(block_number, state);

        let tx_env = prepare_call_env(&block_env, request.clone()).unwrap();
        trace!(?tx_env, "TxEnv is prepared");

        let cfg = self.cfg_infallible(state);
        let cfg_env = get_cfg_env(&block_env, cfg, Some(get_cfg_env_template()));

        let request_gas = request.gas;
        let request_gas_price = request.gas_price;
        let env_gas_limit = block_env.gas_limit;

        // get the highest possible gas limit, either the request's set value or the currently
        // configured gas limit
        let mut highest_gas_limit = request.gas.unwrap_or(env_gas_limit);
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
            .map_err(EthApiError::from)?
            .unwrap_or_default();

        // if the request is a simple transfer, can we optimize?
        if tx_env.data.is_empty() {
            if let TransactTo::Call(to) = tx_env.kind {
                let to_account = self
                    .accounts
                    .get(&to, state)
                    .unwrap_infallible()
                    .map(|account| account.0)
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
        if tx_env.gas_price > 0 {
            // allowance is (balance - tx.value) / tx.gas_price
            let allowance =
                ((account.balance - tx_env.value).to::<u128>() / tx_env.gas_price) as u64;

            if highest_gas_limit > allowance {
                // cap the highest gas limit by max gas caller can afford with a given gas price
                highest_gas_limit = allowance;
            }
        }

        // if the provided gas limit is less than the computed cap, use that
        block_env.gas_limit = std::cmp::min(tx_env.gas_limit, highest_gas_limit);
        trace!(?block_env, "Block env is configured");

        let evm_db = self.get_db(state);

        // execute the call without writing to db
        let result = executor::call(evm_db, &block_env, tx_env.clone(), cfg_env.clone());

        // Exceptional case: init used too much gas, we need to increase the gas limit and try
        // again
        if let Err(EVMError::Transaction(InvalidTransaction::CallerGasLimitMoreThanBlock)) = result
        {
            // if price or limit was included in the request, then we can execute the request
            // again with the block's gas limit to check if revert is gas related or not
            if request_gas.is_some() || request_gas_price.is_some() {
                let evm_db = self.get_db(state);
                return Err(eth_api_into_rpc_error(map_out_of_gas_err(
                    block_env, tx_env, cfg_env, evm_db,
                )));
            }
        }

        let result = match result {
            Ok(result) => match result {
                ExecutionResult::Success { .. } => result,
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
                            block_env, tx_env, cfg_env, evm_db,
                        )))
                    } else {
                        // the transaction did revert
                        Err(invalid_tx_into_rpc_error(
                            RpcInvalidTransactionError::Revert(RevertError::new(output)),
                        ))
                    };
                }
            },
            Err(err) => return Err(eth_api_into_rpc_error(eth_from_evm_error(err))),
        };

        let gas_limit = self.bin_search_gas_limit(
            &cfg_env,
            &block_env,
            &tx_env,
            result,
            highest_gas_limit,
            state,
        )?;

        debug!(
            %gas_limit,
            "EVM module JSON-RPC response from `eth_estimateGas`"
        );
        Ok(U64::from(gas_limit))
    }
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    fn get_maybe_sealed_block(
        &self,
        tx: &TransactionSignedAndRecovered,
        state: &mut ApiStateAccessor<S>,
    ) -> MaybeSealedBlock {
        let block = self.blocks.get(&tx.block_number, state).unwrap_infallible();
        if let Some(block) = block {
            return MaybeSealedBlock::Sealed(block.into());
        }
        let current_block_env = self
            .block_env
            .get(state)
            .unwrap_infallible()
            .unwrap_or_default();
        let block_num: u64 = current_block_env
            .number
            .try_into()
            .expect("Block number is too large to fit in a u64. It's over!");
        assert_eq!(block_num, tx.block_number, "Transaction is in a block that is not yet sealed, but that block is not yet pending! This is impossible!");

        let head = self.head.get(state).unwrap_infallible().unwrap();
        let first_tx_index = head.transactions.end;

        MaybeSealedBlock::Pending {
            block_number: tx.block_number,
            first_tx_number: first_tx_index,
            base_fee_per_gas: current_block_env.basefee,
        }
    }

    fn get_sealed_block_by_number(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> SealedBlock {
        // safe, finalized, and pending are not supported
        match block_number {
            Some(ref block_number) if block_number == "earliest" => {
                let block_numbers = self.block_numbers.get(state).unwrap_infallible().unwrap();
                let first_block_number = block_numbers.start();

                self.blocks
                    .get(first_block_number, state)
                    .unwrap_infallible()
                    .expect("Block must be set")
            }
            Some(ref block_number) if block_number == "latest" => {
                let block_numbers = self.block_numbers.get(state).unwrap_infallible().unwrap();
                let last_block_number = block_numbers.end();

                self.blocks
                    .get(last_block_number, state)
                    .unwrap_infallible()
                    .expect("Block must be set")
            }
            Some(ref block_number) => {
                // hex representation may have 0x prefix
                let block_number = u64::from_str_radix(block_number.trim_start_matches("0x"), 16)
                    .expect("Block number must be a valid hex number, with or without 0x prefix");

                self.blocks
                    .get(&block_number, state)
                    .unwrap_infallible()
                    .expect("Block must be set")
            }
            None => self.get_sealed_block_by_number(Some("latest".into()), state),
        }
    }

    fn resolve_block_env(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> BlockEnv {
        match block_number {
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
        }
    }

    fn bin_search_gas_limit(
        &self,
        cfg_env: &CfgEnv,
        block_env: &BlockEnv,
        tx_env: &TxEnv,
        result: ExecutionResult,
        highest_gas_limit: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<u64, ErrorObjectOwned> {
        // at this point, we know the call succeeded but want to find the _best_ (lowest) gas the
        // transaction succeeds with.
        // we find this by doing a binary search over the
        // possible range NOTE: this is the gas the transaction used, which is less than the
        // transaction requires succeeding
        let gas_used = result.gas_used();
        // the lowest value is capped by the gas it takes for a transfer
        let mut lowest_gas_limit = if tx_env.kind.is_create() {
            MIN_CREATE_GAS
        } else {
            MIN_TRANSACTION_GAS
        };
        let mut highest_gas_limit: u64 = highest_gas_limit;
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
            let result = executor::call(evm_db, block_env, tx_env.clone(), cfg_env.clone());

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
                Ok(result) => match result {
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
                    return Err(eth_api_into_rpc_error(eth_from_evm_error(err)));
                }
            };

            // new midpoint
            mid_gas_limit = ((highest_gas_limit as u128 + lowest_gas_limit as u128) / 2) as u64;
        }
        Ok(highest_gas_limit)
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
    cfg_env.limit_contract_code_size = None;
    cfg_env
}

// modified from: https://github.com/paradigmxyz/reth many times
pub(crate) fn build_rpc_receipt(
    block: MaybeSealedBlock,
    tx: TransactionSignedAndRecovered,
    tx_number: u64,
    receipt: Receipt,
) -> TransactionReceipt {
    let transaction: Recovered<TransactionSigned> = tx.into();
    let from = transaction.signer();

    let block_hash = block.hash();
    let block_number = Some(block.number());
    let transaction_hash = Some(*transaction.hash());
    // Safety: The transaction cannot have a lower number than the block start
    let transaction_index = tx_number
        .checked_sub(block.transactions_start())
        .expect("Overflow while subtracting block start from tx number. This is a bug!");

    let logs: Vec<Log> = receipt
        .receipt
        .logs
        .iter()
        .enumerate()
        .map(|(tx_log_idx, log)| Log {
            inner: log.clone(),
            block_hash,
            block_number,
            block_timestamp: block.timestamp(),
            transaction_hash,
            transaction_index: Some(transaction_index),
            log_index: Some(receipt.log_index_start + tx_log_idx as u64),
            removed: false,
        })
        .collect();

    let logs_bloom = receipt.receipt.bloom();

    let rpc_receipt = alloy_rpc_types::Receipt {
        status: receipt.receipt.success.into(),
        cumulative_gas_used: receipt.receipt.cumulative_gas_used,
        logs,
    };

    let (contract_address, to) = match transaction.kind() {
        TxKind::Create => (Some(from.create(transaction.nonce())), None),
        TxKind::Call(addr) => (None, Some(Address(*addr))),
    };

    TransactionReceipt {
        inner: ReceiptEnvelope::Eip1559(ReceiptWithBloom::new(rpc_receipt, logs_bloom)),
        transaction_hash: *transaction.hash(),
        transaction_index: Some(transaction_index),
        block_hash,
        block_number,
        gas_used: receipt.gas_used,
        effective_gas_price: transaction.effective_gas_price(Some(block.base_fee_per_gas())),
        blob_gas_used: None,
        blob_gas_price: None,
        from,
        to,
        contract_address,
    }
}

fn map_out_of_gas_err<Ws: InfallibleStateAccessor, S: Spec>(
    block_env: BlockEnv,
    mut tx_env: TxEnv,
    cfg_env: CfgEnv,
    db: EvmDb<Ws, S>,
) -> EthApiError
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    let req_gas_limit = tx_env.gas_limit;
    tx_env.gas_limit = block_env.gas_limit;
    let res = executor::call(db, &block_env, tx_env, cfg_env).unwrap();
    match res {
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

fn eth_from_evm_error<Ws: StateAccessor>(err: EVMError<crate::db::Error<Ws>>) -> EthApiError {
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
        RpcInvalidTransactionError::other(ErrorObject::owned(
            -32603,
            format!("Database error: {err}"),
            None::<()>,
        ))
        .into()
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
