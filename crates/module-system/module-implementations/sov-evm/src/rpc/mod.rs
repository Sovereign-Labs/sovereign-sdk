use alloy_consensus::Sealed;
use alloy_consensus::{transaction::Recovered, Transaction as TransactionTrait, TxReceipt};
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{Address, BlockNumber};
use alloy_primitives::{Bytes, TxKind, B256, U256};
use alloy_rpc_types::{
    Block, BlockTransactions, Log, ReceiptEnvelope, ReceiptWithBloom, Transaction,
    TransactionReceipt, TransactionRequest,
};
use alloy_rpc_types::{BlockTransactionsKind, Header};
use alloy_rpc_types_trace::geth::GethDebugTracingOptions;
use alloy_rpc_types_trace::geth::GethTrace;
use alloy_rpc_types_trace::geth::{GethDebugBuiltInTracerType, GethDebugTracerType};
use jsonrpsee::core::RpcResult;
use revm::context::result::ResultAndState;
use revm::context::{BlockEnv, CfgEnv, TxEnv};
use revm_inspectors::tracing::{TracingInspector, TracingInspectorConfig};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rollup_interface::common::RollupHeight;
use sov_rpc_eth_types::{EthApiError, RpcInvalidTransactionError};

use crate::db::EvmDb;
use crate::error::into_rpc_error;
use crate::evm::executor;
use crate::evm::primitive_types::{Receipt, TransactionSigned, TxSignedAndRecovered};
use crate::executor::{get_cfg_env, inspect};
use crate::helpers::{from_recovered_with_block_context, prepare_call_env};
pub use crate::primitive_types::MaybeSealedBlock;
use crate::Evm;

pub(crate) mod error;
pub(crate) mod handlers;

/// Result of String => BlockNr conversion
#[derive(Debug)]
pub enum PendingOrBlock {
    /// Pending block.
    Pending,
    /// Block number.
    Number(u64),
    /// Invalid block number.
    Invalid(String),
}

const ABSOLUTE_MARGIN: u64 = 100_000;
/// gas * 1.5 + 100_000
pub(crate) fn apply_margins(gas: u64) -> Result<u64, RpcInvalidTransactionError> {
    (gas / 2)
        .checked_mul(3)
        .and_then(|with_relative_margin| with_relative_margin.checked_add(ABSOLUTE_MARGIN))
        .ok_or(RpcInvalidTransactionError::GasUintOverflow)
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    fn get_block_transactions(
        &self,
        block: &MaybeSealedBlock,
        kind: BlockTransactionsKind,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<BlockTransactions<Transaction>> {
        let tx_range = block.tx_range();
        let txs = match kind {
            BlockTransactionsKind::Full => {
                let txs = tx_range
                    .clone()
                    .map(|idx| {
                        let tx = self.transactions.get(&idx, state).unwrap_infallible()?;
                        Some(from_recovered_with_block_context(
                            tx.into(),
                            Some(block.hash().unwrap_or_default()),
                            block.number(),
                            U256::from(idx - tx_range.start),
                        ))
                    })
                    .collect::<Option<Vec<_>>>()?;
                BlockTransactions::Full(txs)
            }
            BlockTransactionsKind::Hashes => {
                let hashes = tx_range
                    .into_iter()
                    .map(|idx| {
                        let tx = self.transactions.get(&idx, state).unwrap_infallible()?;
                        Some(*tx.signed_transaction.hash())
                    })
                    .collect::<Option<Vec<_>>>()?;
                BlockTransactions::Hashes(hashes)
            }
        };
        Some(txs)
    }

    fn get_block(
        &self,
        block_number: Option<String>,
        kind: BlockTransactionsKind,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<Block> {
        let block = self.get_sealed_block_by_number(block_number, state)?;
        let hash = block.hash().unwrap_or_default();

        let transactions = self.get_block_transactions(&block, kind, state)?;
        let header = Sealed::new_unchecked(block.header().clone(), hash);
        let header = Header::from_consensus(header, None, None);

        Some(Block {
            header,
            transactions,
            ..Default::default()
        })
    }

    fn get_contract_code(
        &self,
        address: Address,
        mut state: MaybeArchivalState<'_, S>,
    ) -> Option<Bytes> {
        let account = self
            .accounts
            .get(&address, state.deref_mut())
            .unwrap_infallible()?;
        let code = self
            .code
            .get(&account.code_hash, state.deref_mut())
            .unwrap_infallible()?;
        Some(code.bytes())
    }

    fn get_transaction(&self, hash: B256, state: &mut ApiStateAccessor<S>) -> Option<Transaction> {
        let tx_number = self.get_tx_index_by_hash(&hash, state)?;
        let tx = self.transaction(tx_number, state)?;
        let block = self.get_maybe_sealed_block(tx.block_number, state)?;
        let index = U256::from(tx_number - block.transactions_start());
        let tx = from_recovered_with_block_context(tx.into(), block.hash(), block.number(), index);
        Some(tx)
    }

    fn get_receipt_by_hash(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<TransactionReceipt> {
        let number = self.get_tx_index_by_hash(&hash, state)?;
        self.get_receipt_by_index(number, state)
    }

    fn get_receipt_by_index(
        &self,
        number: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<TransactionReceipt> {
        let tx = self.transaction(number, state)?;
        let block = self.get_maybe_sealed_block(tx.block_number, state)?;
        let receipt = self.receipt(number, state)?;
        Some(build_rpc_receipt(block, tx, number, receipt))
    }

    fn get_receipts(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<Vec<TransactionReceipt>> {
        let block = self.get_sealed_block_by_number(block_number, state)?;
        let receipts = block
            .tx_range()
            .map(|index| self.get_receipt_by_index(index, state))
            .collect::<Option<Vec<_>>>()?;
        Some(receipts)
    }

    fn trace_transaction(
        &self,
        block_env: &BlockEnv,
        tx_env: TxEnv,
        cfg: CfgEnv,
        db: &mut EvmDb<ApiStateAccessor<S>, S>,
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
                    let res = inspect(db, &block_env, tx_env, cfg, &mut inspector)?;
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
    ) -> Result<ResultAndState, EthApiError> {
        let block_env = self.resolve_block_env(block_number, state)?;
        let tx_env = prepare_call_env(&block_env, request.clone())?;
        let cfg = self.cfg_infallible(state);
        let cfg_env = get_cfg_env(&block_env, cfg, Some(get_cfg_env_template()));
        let evm_db: EvmDb<_, S> = self.get_db(state);

        Ok(executor::transact(evm_db, &block_env, tx_env, cfg_env)?)
    }

    /// Retrieves a sealed block generated from an existing or pending block.
    pub fn get_maybe_sealed_block(
        &self,
        block_number: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<MaybeSealedBlock> {
        let block = self.blocks.get(&block_number, state).unwrap_infallible();
        if let Some(block) = block {
            return Some(MaybeSealedBlock::Sealed(block));
        }

        let pending = self.pending_block(state);
        if block_number == pending.header.number {
            return Some(MaybeSealedBlock::Pending(pending));
        }

        None
    }

    /// Convert string to block nr.
    pub fn str_to_block_nr(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> PendingOrBlock {
        let block_number_str = block_number.unwrap_or_else(|| "latest".into());

        match block_number_str.as_str() {
            "earliest" => PendingOrBlock::Number(*self.block_numbers(state).start()),
            "latest" => PendingOrBlock::Number(*self.block_numbers(state).end()),
            "pending" => PendingOrBlock::Pending,
            number => match u64::from_str_radix(number.trim_start_matches("0x"), 16) {
                Ok(nr) => PendingOrBlock::Number(nr),
                Err(_) => PendingOrBlock::Invalid(block_number_str),
            },
        }
    }

    /// Converts BlockNumberOrTag into number.
    pub fn resolve_block_number(
        &self,
        block: BlockNumberOrTag,
        state: &mut ApiStateAccessor<S>,
    ) -> BlockNumber {
        let block_numbers = self.block_numbers(state);
        let block_number = match block {
            BlockNumberOrTag::Earliest => *block_numbers.start(),
            BlockNumberOrTag::Latest | BlockNumberOrTag::Finalized | BlockNumberOrTag::Safe => {
                *block_numbers.end()
            }
            BlockNumberOrTag::Number(nr) => nr,
            BlockNumberOrTag::Pending => *block_numbers.end() + 1,
        };
        block_number
    }

    /// Retrieves a sealed block by number.
    pub fn get_sealed_block_by_number(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<MaybeSealedBlock> {
        let pending_or_block_nr = self.str_to_block_nr(block_number, state);

        match pending_or_block_nr {
            PendingOrBlock::Number(nr) => self.get_maybe_sealed_block(nr, state),
            PendingOrBlock::Pending => {
                let pending_block = self.pending_block(state);
                Some(MaybeSealedBlock::Pending(pending_block))
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
            excess_blob_gas: current_block_env
                .blob_excess_gas_and_price
                .map(|blob_gas| blob_gas.excess_blob_gas),
            base_fee_per_gas: Some(current_block_env.basefee),

            ..Default::default()
        };

        crate::Block {
            header,
            transactions: start..end,
        }
    }

    fn resolve_state<'a>(
        &self,
        block_number: Option<String>,
        state: &'a mut ApiStateAccessor<S>,
    ) -> RpcResult<MaybeArchivalState<'a, S>> {
        let state = match block_number {
            None => MaybeArchivalState::Current(state),
            Some(number) if number == "latest" => MaybeArchivalState::Current(state),
            _ => {
                let pending_or_block_nr = self.str_to_block_nr(block_number, state);
                match pending_or_block_nr {
                    PendingOrBlock::Pending => MaybeArchivalState::Current(state),
                    PendingOrBlock::Number(number) => {
                        let archival_state = state
                            .get_archival_state(RollupHeight::new(number))
                            .map_err(into_rpc_error)?;
                        MaybeArchivalState::Archival(archival_state.into())
                    }
                    PendingOrBlock::Invalid(_) => {
                        return Err(EthApiError::UnknownBlockOrTxIndex.into());
                    }
                }
            }
        };
        Ok(state)
    }

    fn resolve_block_env(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<BlockEnv, EthApiError> {
        let maybe_blcok = self
            .get_sealed_block_by_number(block_number, state)
            .ok_or(EthApiError::UnknownBlockOrTxIndex)?;

        Ok(match maybe_blcok {
            MaybeSealedBlock::Pending(_) => self
                .block_env
                .get(state)
                .unwrap_infallible()
                .expect("The impossible happened: block_env is not set."),
            MaybeSealedBlock::Sealed(sealed_block) => BlockEnv::from(sealed_block),
        })
    }
}

use std::ops::{Deref, DerefMut};

enum MaybeArchivalState<'a, S: Spec> {
    Current(&'a mut ApiStateAccessor<S>),
    Archival(Box<ApiStateAccessor<S>>),
}

impl<'a, S: Spec> Deref for MaybeArchivalState<'a, S> {
    type Target = ApiStateAccessor<S>;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Current(a) => a,
            Self::Archival(a) => a,
        }
    }
}

impl<'a, S: Spec> DerefMut for MaybeArchivalState<'a, S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            Self::Current(a) => a,
            Self::Archival(a) => a,
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
    tx: TxSignedAndRecovered,
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
