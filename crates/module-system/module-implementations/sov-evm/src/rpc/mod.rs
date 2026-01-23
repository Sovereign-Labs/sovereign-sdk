use std::ops::DerefMut;

use crate::db::EvmDb;
use crate::error::into_rpc_error;
use crate::evm::executor;
use crate::evm::primitive_types::{Receipt, TransactionSigned, TxSignedAndRecovered};
use crate::executor::get_cfg_env;
use crate::helpers::{from_recovered_with_block_context, prepare_call_env};
pub use crate::primitive_types::MaybeSealedBlock;
use crate::primitive_types::SyntheticBlockWithoutRootsAndBloom;
use crate::{verify_contract_creation_allowlist, Evm, SealedBlock};
use alloy_consensus::{transaction::Recovered, Transaction as TransactionTrait, TxReceipt};
use alloy_consensus::{EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{Address, BlockNumber, Bloom, B64};
use alloy_primitives::{Bytes, TxKind, B256, U256};
use alloy_rpc_types::{
    Block, BlockTransactions, Log, ReceiptEnvelope, ReceiptWithBloom, Transaction,
    TransactionReceipt, TransactionRequest,
};
use alloy_rpc_types::{BlockTransactionsKind, Header};
use maybe_archival_state::MaybeArchivalState;
use revm::context::result::ResultAndState;
use revm::context::{BlockEnv, CfgEnv};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::da::Time;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{ApiStateAccessor, Spec, VersionReader};
use sov_rollup_interface::common::RollupHeight;
use sov_rpc_eth_types::{EthApiError, LogWithExecutionTimestamp, RpcInvalidTransactionError};

pub(crate) mod error;
pub(crate) mod handlers;
pub(crate) mod maybe_archival_state;

mod fee_history;
mod trace;

/// Result of String => BlockNr conversion
#[derive(Debug)]
pub enum PendingOrBlock {
    /// Pending block.
    Pending,
    /// Block number.
    Number(u64),
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
        block: &SealedBlock,
        kind: BlockTransactionsKind,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<BlockTransactions<Transaction>, EthApiError> {
        let tx_range = block.transactions.start..block.transactions.end;
        let txs = match kind {
            BlockTransactionsKind::Full => {
                let txs: Vec<Transaction> = tx_range
                    .into_iter()
                    .enumerate()
                    .map(|(pos, idx)| {
                        let tx = self.tx(idx, state)?;
                        Ok::<_, EthApiError>(from_recovered_with_block_context(
                            tx.into(),
                            Some(block.header.hash()),
                            block.number,
                            pos as u64,
                        ))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                BlockTransactions::Full(txs)
            }
            BlockTransactionsKind::Hashes => {
                let hashes = tx_range
                    .into_iter()
                    .map(|idx| {
                        let tx = self.tx(idx, state)?;
                        Ok::<_, EthApiError>(*tx.signed_transaction.hash())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                BlockTransactions::Hashes(hashes)
            }
        };
        Ok(txs)
    }
  
    fn get_block(
        &self,
        block_number: Option<String>,
        kind: BlockTransactionsKind,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<Option<Block>, EthApiError> {
        let Some(block) = self.get_sealed_block_by_number(block_number, state)? else {
            return Ok(None);
        };
        match block {
            MaybeSealedBlock::Sealed(sealed) =>  {
                let block_size = sealed.rlp_size;
                let transactions = self.get_block_transactions(&sealed, kind, state)?;
                let header = Header::from_consensus(sealed.header.into(), None, Some(U256::from(block_size)));
                Ok(Some(Block {
                    header,
                    transactions,
                    uncles: vec![],
                    withdrawals: None,
                }))
            }
            // For pending blocks, we would like to avoid fetching the whole block body for performance reasons
            MaybeSealedBlock::PartialSynthetic(block) =>  {
                let (header, txs) = self.get_synthetic_block_contents_slow(block, state)?;
                let txs = match kind {
                    BlockTransactionsKind::Full => BlockTransactions::Full(txs.into_iter().enumerate().map(|(tx_idx, tx)| from_recovered_with_block_context(tx.into(), Some(header.hash), header.number, tx_idx as u64)).collect()),
                    BlockTransactionsKind::Hashes => BlockTransactions::Hashes(txs.into_iter().map(|tx| *tx.signed_transaction.hash()).collect()),
                };
                Ok(Some(Block {
                    header,
                    transactions: txs,
                    uncles: vec![],
                    withdrawals: None,
                }))
            }
        }
    }

    /// Populates the header of a partial synthetic block and returns the transactions.
    /// 
    /// This function has to fetch all the transactions and receipts, so it's relatively slow - avoid when possible.
    pub fn get_synthetic_block_contents_slow(
        &self,
        block: SyntheticBlockWithoutRootsAndBloom,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<(Header, Vec<TxSignedAndRecovered>), EthApiError> {
            let len = block.transactions.end - block.transactions.start;
                let mut txs = Vec::with_capacity(len as usize);
                let mut receipts = Vec::with_capacity(len as usize);
                for tx_idx in block.transactions.clone() {
                    let tx = self.tx(tx_idx, state)?;
                    txs.push(tx);
                    let receipt = self.receipt(tx_idx, state).unwrap_or_else(|| panic!("Tx {tx_idx} exists but has no corresponding receipt. This is a bug, please report it."));
                    let receipt = receipt.0.receipt;
                    receipts.push(receipt);
                }
                let (synthetic_block, txs) = block.finish_and_seal(txs, &receipts);
                let header = Header::from_consensus(synthetic_block.header, None, Some(U256::from(synthetic_block.rlp_size)));
            Ok((header, txs))
       
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
        let tx_number = self.tx_index(&hash, state)?;
        let tx = self.transaction(tx_number, state)?;
        let block = self.get_maybe_sealed_block(tx.block_number, state)?;
        let index = tx_number - block.transactions_start();
        let tx = from_recovered_with_block_context(tx.into(), block.hash(), block.number(), index);
        Some(tx)
    }

    fn get_receipt_by_hash(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>> {
        let number = self.tx_index(&hash, state)?;
        self.get_receipt_by_index(number, state)
    }

    fn get_receipt_by_index(
        &self,
        number: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>> {
        let tx = self.transaction(number, state)?;
        let block = self.get_maybe_sealed_block(tx.block_number, state)?;
        let (receipt, time) = self.receipt(number, state)?;
        Some(build_rpc_receipt(block, tx, number, receipt, time))
    }

    fn get_receipts(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<
        Option<Vec<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>>>,
        EthApiError,
    > {
        let Some(block) = self.get_sealed_block_by_number(block_number, state)? else {
            return Ok(None);
        };
        let Some(receipts) = block
            .tx_range()
            .map(|index| self.get_receipt_by_index(index, state))
            .collect::<Option<Vec<_>>>()
        else {
            return Ok(None);
        };
        Ok(Some(receipts))
    }

    fn call(
        &self,
        request: TransactionRequest,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<ResultAndState, EthApiError> {
        let block_env = self.resolve_block_env_for_call(block_number, state)?;
        let tx_env = prepare_call_env(&block_env, request.clone())?;
        let caller = tx_env.caller;
        let cfg = self.cfg_infallible(state);
        let cfg_env = get_cfg_env(&block_env, &cfg, Some(get_cfg_env_template()));
        let mut evm_db: EvmDb<_, S> = self.db(state);
        let result = executor::transact(&mut evm_db, &block_env, tx_env, cfg_env)?;
        verify_contract_creation_allowlist(&result.state, &caller, &cfg, &mut evm_db)
            .map_err(|e| EthApiError::other(into_rpc_error(e)))?;
        Ok(result)
    }

    /// Retrieves a sealed block generated from an existing or pending block.
    pub fn get_maybe_sealed_block(
        &self,
        block_number: u64,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<MaybeSealedBlock> {
        // Check if the block is already sealed. Important: We must do this before checking the block env, because blocks are sealed in the finalize_hook.
        // but the block env is set during the begin_rollup_block_hook.
        // That means that if we call this function after the finalize hook but before the start of the next block, the "pending" block env corresponds to the same
        // number and set of txs as the latest sealed block - in which case we want to return the sealed version of it rather than making up a synthetic one.
        let block = self.blocks.get(&block_number, state).unwrap_infallible();
        if let Some(block) = block {
            return Some(MaybeSealedBlock::Sealed(block));
        }

        let block_env = self.block_env(state).unwrap_infallible();
        if block_env.number == block_number {
            let pending = self.pending_block(Some(block_env), state);
            return Some(MaybeSealedBlock::PartialSynthetic(pending));
        }

        None
    }

    /// Convert string to block nr.
    pub fn str_to_block_nr(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<PendingOrBlock, EthApiError> {
        let block_number_str = block_number.unwrap_or_else(|| "latest".into());

        Ok(match block_number_str.as_str() {
            "earliest" => PendingOrBlock::Number(*self.block_numbers(state).start()),
            // We treat latest and pending the same to avoid foundry issues
            "latest" | "pending" => PendingOrBlock::Pending,
            number => {
                let number = u64::from_str_radix(number.trim_start_matches("0x"), 16)
                    .map_err(|e| EthApiError::InvalidBlockNumber(block_number_str, e))?;
                PendingOrBlock::Number(number)
            }
        })
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
            BlockNumberOrTag::Finalized | BlockNumberOrTag::Safe => *block_numbers.end(),
            BlockNumberOrTag::Number(nr) => nr,
            // We treat latest and pending the same to avoid foundry issues
            BlockNumberOrTag::Latest | BlockNumberOrTag::Pending => *block_numbers.end() + 1,
        };
        block_number
    }

    /// Retrieves a sealed block by number.
    pub fn get_sealed_block_by_number(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<Option<MaybeSealedBlock>, EthApiError> {
        let pending_or_block_nr = self.str_to_block_nr(block_number, state)?;

        Ok(match pending_or_block_nr {
            PendingOrBlock::Number(nr) => self.get_maybe_sealed_block(nr, state),
            PendingOrBlock::Pending => {
                let pending_block = self.pending_block(None, state);
                Some(MaybeSealedBlock::PartialSynthetic(pending_block))
            }
        })
    }

    /// Retrieves the latest block.
    pub fn latest_block(&self, state: &mut ApiStateAccessor<S>) -> SealedBlock {
        let block_numbers = self.block_numbers(state);
        self.blocks
            .get(block_numbers.end(), state)
            .unwrap_infallible()
            .expect("Block should exist as index is inside block_numbers")
    }

    /// Retrieves the pending block. We maintain the invariant that the pending block always has number
    /// latest_sealed_block.number + 1. (Both are updated during the finalize_hook, so they're atomic).
    /// 
    /// Note that values in the block_env (including the block number there!) are updated during the begin_rollup_block_hook, so the values here may be stale
    /// if this function is called while no rollup block is in progress. In that case, the number of transactions will be zero.
    // TODO: Have this function return None if no block is pending!
    pub fn pending_block(&self, block_env: Option<BlockEnv>, state: &mut ApiStateAccessor<S>) -> SyntheticBlockWithoutRootsAndBloom {
        let block_numbers = self.block_numbers(state);

        let head_block = self
            .blocks
            .get(block_numbers.end(), state)
            .unwrap_infallible()
            .expect("Block should exist as index is inside block_numbers");
        let current_block_env = block_env.unwrap_or_else(|| self.block_env(state).unwrap_infallible());

        assert_eq!(&head_block.header.number, block_numbers.end());

        let pending_transactions_len = self.pending_transactions.len(state).unwrap_infallible();

        let start = head_block.transactions.end;
        let end = start + pending_transactions_len;

        let pending_block_number = head_block.header.number + 1;

        let header = alloy_consensus::Header {
            parent_hash: head_block.header.seal(),
            number: pending_block_number,
            timestamp: current_block_env.timestamp.to::<u64>(),
            excess_blob_gas: current_block_env
                .blob_excess_gas_and_price
                .map(|blob_gas| blob_gas.excess_blob_gas),
            base_fee_per_gas: Some(current_block_env.basefee),
            gas_limit: current_block_env.gas_limit,
            // Values that will be filled in later
            gas_used: 0,
            logs_bloom: Bloom::default(),
            state_root: EMPTY_ROOT_HASH,
            transactions_root: EMPTY_ROOT_HASH,
            receipts_root: EMPTY_ROOT_HASH,

            // Values that never need to be initailized
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: Address::ZERO,
            difficulty: U256::ZERO,
            extra_data: Bytes::default(),
            mix_hash: B256::ZERO,
            nonce: B64::ZERO,
            withdrawals_root: None,
            blob_gas_used: None,
            parent_beacon_block_root: None,
            requests_hash: None,
        };

        SyntheticBlockWithoutRootsAndBloom::new(header, start..end)
    }

    fn resolve_state<'a>(
        &self,
        block_number: Option<String>,
        state: &'a mut ApiStateAccessor<S>,
    ) -> Result<MaybeArchivalState<'a, S>, EthApiError> {
        let pending_or_block_nr = self.str_to_block_nr(block_number, state)?;
        match pending_or_block_nr {
            PendingOrBlock::Pending => Ok(MaybeArchivalState::Current(state)),
            PendingOrBlock::Number(number) => {
                if number == state.rollup_height_to_access().get()
                    || (number == state.rollup_height_to_access().get() + 1)
                {
                    return Ok(MaybeArchivalState::Current(state));
                }
                let archival_state = state.get_archival_state(RollupHeight::new(number))?;
                Ok(MaybeArchivalState::Archival(archival_state.into()))
            }
        }
    }

    fn resolve_block_env_for_call(
        &self,
        block_number: Option<String>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<BlockEnv, EthApiError> {
        let maybe_block = self
            .get_sealed_block_by_number(block_number, state)?
            .ok_or(EthApiError::UnknownBlock)?;

        let mut block_env = match maybe_block {
            MaybeSealedBlock::PartialSynthetic(_) => self.block_env(state).unwrap_infallible(),
            MaybeSealedBlock::Sealed(sealed_block) => BlockEnv::from(sealed_block),
        };
        // Set the base fee to zero for evm execution. Gas is paid for by the sov gas meter instead
        block_env.basefee = 0;
        Ok(block_env)
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
    cfg_env.memory_limit = 50 * 1024 * 1024; // 50MB
    cfg_env
}

// modified from: https://github.com/paradigmxyz/reth many times
pub(crate) fn build_rpc_receipt(
    block: MaybeSealedBlock,
    tx: TxSignedAndRecovered,
    tx_number: u64,
    receipt: Receipt,
    time: Time,
) -> TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>> {
    let transaction: Recovered<TransactionSigned> = tx.into();
    let from = transaction.signer();

    let block_hash = block.hash();
    let block_number = Some(block.number());
    let transaction_index = tx_number
        .checked_sub(block.tx_range().start)
        .expect("tx_number is within block.tx_range()");

    let transaction_hash = receipt.transaction_hash;
    let logs_bloom = receipt.receipt.bloom();

    let time = time.as_millis().try_into().unwrap_or_default();
    let logs: Vec<LogWithExecutionTimestamp> = receipt
        .receipt
        .logs
        .into_iter()
        .enumerate()
        .map(|(tx_log_idx, log)| LogWithExecutionTimestamp {
            log: Log {
                inner: log,
                block_hash,
                block_number,
                block_timestamp: Some(block.timestamp()),
                transaction_hash: Some(transaction_hash),
                transaction_index: Some(transaction_index),
                log_index: Some(receipt.log_index_start + tx_log_idx as u64),
                removed: false,
            },
            time_executed_ms: time,
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
