use std::error::Error;

use alloy_consensus::{transaction::Recovered, Transaction as TransactionTrait, TxReceipt};
use alloy_primitives::{Address, U64};
use alloy_primitives::{Bytes, TxKind, B256, U256};
use alloy_rpc_types::{
    state::StateOverride, Block, BlockOverrides, BlockTransactions, FeeHistory, Log,
    ReceiptEnvelope, ReceiptWithBloom, Transaction, TransactionReceipt, TransactionRequest,
};
use alloy_rpc_types_trace::geth::GethDebugTracingOptions;
use alloy_rpc_types_trace::geth::GethTrace;
use alloy_rpc_types_trace::geth::{GethDebugBuiltInTracerType, GethDebugTracerType};
use error::ensure_success;
use jsonrpsee::core::RpcResult;
use jsonrpsee::types::{ErrorObject, ErrorObjectOwned};
use revm::context::result::{EVMError, ExecutionResult, InvalidHeader};
use revm::context::{BlockEnv, CfgEnv, TxEnv};
use revm::Database;
use revm_inspectors::tracing::{TracingInspector, TracingInspectorConfig};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::{config_value, rpc_gen};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, GasMeter, GasSpec, Spec, StateAccessor};
use sov_rollup_interface::common::RollupHeight;
use sov_rpc_eth_types::{EthApiError, RpcInvalidTransactionError};
use tracing::debug;

use crate::conversions::replay_tx_env;
use crate::db::EvmDb;
use crate::evm::executor;
use crate::evm::primitive_types::{
    Receipt, SealedBlock, TransactionSigned, TransactionSignedAndRecovered,
};
use crate::executor::{get_cfg_env, inspect, transact_commit};
use crate::helpers::{
    from_primitive_with_hash, from_recovered_with_block_context, prepare_call_env,
};
pub use crate::primitive_types::MaybeSealedBlock;
use crate::Evm;

pub(crate) mod error;

#[rpc_gen(client, server)]
impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    /// Handler for `net_version`
    #[rpc_method(name = "net_version")]
    pub fn net_version(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<String> {
        debug!("EVM module JSON-RPC request to `net_version`");

        // Network ID is the same as chain ID for most networks
        let chain_id = config_value!("CHAIN_ID");
        Ok(chain_id.to_string())
    }

    /// Handler for: `eth_chainId`
    #[rpc_method(name = "eth_chainId")]
    pub fn chain_id(&self, _state: &mut ApiStateAccessor<S>) -> RpcResult<Option<U64>> {
        let chain_id = config_value!("CHAIN_ID");
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

        let maybe_block = || -> Option<Block> {
            let block = self.get_sealed_block_by_number(block_number, state)?;
            let header = from_primitive_with_hash(block.header.clone());

            let transactions = if Some(true) == details {
                BlockTransactions::Full(
                    block
                        .transactions
                        .clone()
                        .map(|index| {
                            let tx = self.transactions.get(&index, state).unwrap_infallible()?;
                            Some(from_recovered_with_block_context(
                                tx.into(),
                                Some(block.header.seal()),
                                block.header.number,
                                U256::from(index - block.transactions.start),
                            ))
                        })
                        .collect::<Option<Vec<_>>>()?,
                )
            } else {
                BlockTransactions::Hashes(
                    block
                        .transactions
                        .clone()
                        .map(|index| {
                            let tx = self.transactions.get(&index, state).unwrap_infallible()?;
                            Some(*tx.signed_transaction.hash())
                        })
                        .collect::<Option<Vec<_>>>()?,
                )
            };

            Some(Block {
                header,
                transactions,
                ..Default::default()
            })
        };

        Ok(maybe_block())
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
            .map_err(|e| eth_api_into_rpc_error(EthApiError::from(e)))?
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

        Ok(code.bytecode().clone())
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
        let mut maybe_tx = || -> Option<Transaction> {
            let tx_number = self.get_tx_index_by_hash(&hash, state)?;
            let tx = self.transaction(tx_number, state)?;
            let block = self.get_maybe_sealed_block(tx.block_number, state);

            Some(from_recovered_with_block_context(
                tx.into(),
                block.hash(),
                block.number(),
                U256::from(tx_number - block.transactions_start()),
            ))
        };

        let transaction = maybe_tx();
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

        let mut maybe_receipt = || -> Option<TransactionReceipt> {
            let number = self.get_tx_index_by_hash(&hash, state)?;
            let tx = self.transaction(number, state)?;
            let block = self.get_maybe_sealed_block(tx.block_number, state);
            let receipt = self.receipt(number, state)?;
            Some(build_rpc_receipt(block, tx, number, receipt))
        };

        Ok(maybe_receipt())
    }

    /// Handler for: `eth_call`
    //https://github.com/paradigmxyz/reth/blob/f577e147807a783438a3f16aad968b4396274483/crates/rpc/rpc/src/eth/api/transactions.rs#L502
    #[rpc_method(name = "eth_call")]
    pub fn eth_call(
        &self,
        request: TransactionRequest,
        block_number: Option<String>,
        _state_overrides: Option<StateOverride>,
        _block_overrides: Option<Box<BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<Bytes> {
        debug!("EVM module JSON-RPC request to `eth_call`");
        let result = self.call(request, block_number, state)?;
        ensure_success(result).map_err(eth_api_into_rpc_error)
    }

    /// Handler for: `eth_blockNumber`
    #[rpc_method(name = "eth_blockNumber")]
    pub fn block_number(&self, state: &mut ApiStateAccessor<S>) -> RpcResult<U256> {
        let block_number_range = self
            .block_numbers
            .get(state)
            .unwrap_infallible()
            // Justified, we set it at genesis and later only override it.
            .expect("The impossible happened: block_numbers was not set.");

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
        let result = self.call(request, block_number, state)?;
        let gas_used = result.gas_used();
        state
            .charge_linear_gas(
                &<S as GasSpec>::gas_to_charge_per_evm_gas(),
                gas_used as u32,
            )
            .unwrap();
        let gas_meter = state.try_as_basic_gas_meter().unwrap();
        let total_gas_used =
            gas_meter.initial_gas.as_ref()[0] - gas_meter.remaining_gas.as_ref()[0];
        const RELATIVE_MARGIN: u64 = 100_000;
        let gas_used_with_margins = (total_gas_used * 3) / 2 + RELATIVE_MARGIN; // gas * 1.5 + 100_000
        Ok(U64::from(gas_used_with_margins))
    }

    /// Handler for: `debug_traceTransaction`
    #[rpc_method(name = "debug_traceTransaction")]
    pub fn debug_trace_transaction(
        &self,
        tx_hash: B256,
        opts: Option<GethDebugTracingOptions>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<GethTrace> {
        // Get transaction and block data
        let index = self
            .get_tx_index_by_hash(&tx_hash, state)
            .ok_or_else(|| eth_api_into_rpc_error(EthApiError::PrunedHistoryUnavailable))?;

        let traced_tx = self
            .transaction(index, state)
            .ok_or_else(|| eth_api_into_rpc_error(EthApiError::PrunedHistoryUnavailable))?;

        let block = self
            .blocks
            .get(&traced_tx.block_number, state)?
            .expect("Transaction block not available");

        // Get archival state and environment
        let mut archival_state = state
            .get_archival_state(RollupHeight::new(traced_tx.block_number - 1))
            .map_err(into_rpc_error)?;

        let block_env = self
            .block_env(&mut archival_state)?
            .ok_or_else(|| eth_api_into_rpc_error(EthApiError::PrunedHistoryUnavailable))?;

        let cfg = self.cfg(&mut archival_state).map_err(into_rpc_error)?;
        let cfg_env = get_cfg_env(&block_env, cfg, None);

        // Replay previous transactions in the block
        let mut evm_db = self.get_db(&mut archival_state);

        for tx_idx in block.transactions {
            let tx = self
                .transaction(tx_idx, state)
                .ok_or_else(|| eth_api_into_rpc_error(EthApiError::PrunedHistoryUnavailable))?;

            // Skip the transaction we're tracing
            if *tx.signed_transaction.hash() == tx_hash {
                continue;
            }

            transact_commit(
                &mut evm_db,
                block_env.clone(),
                replay_tx_env(&tx),
                cfg_env.clone(),
            )
            .map_err(|e| eth_api_into_rpc_error(eth_from_evm_error(e)))?;
        }

        // Trace the target transaction
        self.trace_transaction(
            block_env.clone(),
            replay_tx_env(&traced_tx),
            cfg_env,
            evm_db,
            &opts.unwrap_or_default(),
        )
        .map_err(eth_api_into_rpc_error)
    }
}

/// Result of String => BlockNr conversion
pub enum PendingOrBlock {
    /// Pending blcock.
    Pending,
    /// Block number.
    Number(u64),
    /// Invalid block number.
    Invalid(String),
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    fn trace_transaction(
        &self,
        block_env: BlockEnv,
        tx_env: TxEnv,
        cfg: CfgEnv,
        db: EvmDb<ApiStateAccessor<S>, S>,
        opts: &GethDebugTracingOptions,
    ) -> Result<GethTrace, EthApiError> {
        let GethDebugTracingOptions {
            tracer,
            tracer_config,
            ..
        } = opts;
        if let Some(tracer) = tracer {
            return match tracer {
                GethDebugTracerType::BuiltInTracer(GethDebugBuiltInTracerType::CallTracer) => {
                    let call_config = tracer_config
                        .clone()
                        .into_call_config()
                        .map_err(|_| EthApiError::InvalidTracerConfig)?;

                    let inspector_config =
                        TracingInspectorConfig::from_geth_call_config(&call_config);
                    let mut inspector = TracingInspector::new(inspector_config);

                    let gas_limit = tx_env.gas_limit;
                    let res = inspect(db, block_env, tx_env, cfg, &mut inspector)?;
                    inspector.set_transaction_gas_limit(gas_limit);

                    let frame = inspector
                        .geth_builder()
                        .geth_call_traces(call_config, res.result.gas_used());

                    return Ok(frame.into());
                }
                _ => Err(EthApiError::Unsupported("unsupported tracer")),
            };
        };
        Err(EthApiError::Unsupported("unsupported tracer"))
    }

    fn call(
        &self,
        request: TransactionRequest,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> RpcResult<ExecutionResult> {
        let Some(block_env) = self.resolve_block_env(block_number, state) else {
            return Err(eth_api_into_rpc_error(EthApiError::UnknownBlockOrTxIndex));
        };
        let tx_env =
            prepare_call_env(&block_env, request.clone()).map_err(eth_api_into_rpc_error)?;
        let cfg = self.cfg_infallible(state);
        let cfg_env = get_cfg_env(&block_env, cfg, Some(get_cfg_env_template()));
        let evm_db: EvmDb<_, S> = self.get_db(state);

        executor::call(evm_db, block_env, tx_env, cfg_env)
            .map_err(|err| eth_api_into_rpc_error(eth_from_evm_error(err)))
    }

    /// Retrieves a sealed block generated from an existing or pending block.
    pub fn get_maybe_sealed_block(
        &self,
        block_number: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> MaybeSealedBlock {
        let block = self.blocks.get(&block_number, state).unwrap_infallible();
        if let Some(block) = block {
            return MaybeSealedBlock::Sealed(block);
        }

        let pending = self.pending_block(state);
        MaybeSealedBlock::Pending(pending)
    }

    /// Convert string to block nr.
    pub fn str_to_block_nr(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> PendingOrBlock {
        let block_number_str = block_number.unwrap_or_else(|| "latest".into());

        match block_number_str.as_str() {
            "earliest" => {
                let block_numbers = self
                    .block_numbers
                    .get(state)
                    .unwrap_infallible()
                    // This is justified, as block numbers are set at genesis and only overridden later.
                    .expect("The impossible happened: block_numbers was not set.");

                PendingOrBlock::Number(*block_numbers.start())
            }
            "latest" => {
                let block_numbers = self
                    .block_numbers
                    .get(state)
                    .unwrap_infallible()
                    // This is justified, as block numbers are set at genesis and only overridden later.
                    .expect("The impossible happened: block_numbers was not set.");

                PendingOrBlock::Number(*block_numbers.end())
            }

            "pending" => PendingOrBlock::Pending,
            number => match u64::from_str_radix(number.trim_start_matches("0x"), 16) {
                Ok(nr) => PendingOrBlock::Number(nr),
                Err(_) => PendingOrBlock::Invalid(block_number_str),
            },
        }
    }

    /// Retrieves a sealed block by number.
    pub fn get_sealed_block_by_number(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<SealedBlock> {
        let pending_or_block_nr = self.str_to_block_nr(block_number, state);

        match pending_or_block_nr {
            PendingOrBlock::Number(nr) => self.blocks.get(&nr, state).unwrap_infallible(),
            PendingOrBlock::Pending => {
                let pending_block = self.pending_block(state);
                Some(pending_block.seal())
            }
            PendingOrBlock::Invalid(invalid) => {
                tracing::error!(invalid, "Invalid block number");
                None
            }
        }
    }

    /// Retrieves the pending block.
    pub fn pending_block(&self, state: &mut ApiStateAccessor<S>) -> crate::Block {
        let block_numbers = self
            .block_numbers
            .get(state)
            .unwrap_infallible()
            // This is justified, as block numbers are set at genesis and only overridden later.
            .expect("The impossible happened: block_numbers was not set.");

        let head_block = self
            .blocks
            .get(block_numbers.end(), state)
            .unwrap_infallible()
            // This is justified, as we just fetched `block_numbers`.
            .expect("The impossible happened: parent_block was not set.");

        let current_block_env = self
            .block_env
            .get(state)
            .unwrap_infallible()
            .unwrap_or_default();

        assert_eq!(&head_block.header.number, block_numbers.end());

        let pending_transactions_len = self.pending_transactions.len(state).unwrap_infallible();

        let start = head_block.transactions.end;
        let end = start + pending_transactions_len;

        let pending_block_number = head_block.header.number + 1;

        let header = alloy_consensus::Header {
            parent_hash: head_block.header.seal(),
            number: pending_block_number,
            timestamp: current_block_env
                .timestamp
                .try_into()
                .expect("The impossible happened: timestamp overflow u64"),
            ..Default::default()
        };

        crate::Block {
            header,
            transactions: start..end,
        }
    }

    fn resolve_block_env(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<BlockEnv> {
        match block_number {
            Some(ref block_number) if block_number == "pending" => {
                self.block_env.get(state).unwrap_infallible()
            }
            _ => {
                let block = self.get_sealed_block_by_number(block_number, state)?;
                Some(BlockEnv::from(block))
            }
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
    // Safety: The transaction cannot have a lower number than the block start
    let transaction_index = tx_number
        .checked_sub(block.transactions_start())
        .expect("The impossible happened: overflow while subtracting block start from tx number.");

    let transaction_hash = receipt.transaction_hash;
    let logs_bloom = receipt.receipt.bloom();

    let logs: Vec<Log> = receipt
        .receipt
        .logs
        .into_iter()
        .enumerate()
        .map(|(tx_log_idx, log)| Log {
            inner: log,
            block_hash,
            block_number,
            block_timestamp: Some(block.timestamp()),
            transaction_hash: Some(transaction_hash),
            transaction_index: Some(transaction_index),
            log_index: Some(receipt.log_index_start + tx_log_idx as u64),
            removed: false,
        })
        .collect();

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
        transaction_hash,
        transaction_index: Some(transaction_index),
        block_hash,
        block_number,
        gas_used: receipt.gas_used,
        effective_gas_price: 0,
        blob_gas_used: None,
        blob_gas_price: None,
        from,
        to,
        contract_address,
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
        EthApiError::EvmCustom(format!("Database error: {err}"))
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

/// Converts internal error into rpc error
pub fn into_rpc_error(err: impl Error) -> ErrorObjectOwned {
    ErrorObject::owned(500, format!("{err:?}"), None::<()>)
}
