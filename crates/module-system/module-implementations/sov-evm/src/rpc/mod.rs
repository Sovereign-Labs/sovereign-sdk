use std::any::Any;
use std::collections::hash_map::Entry;
#[cfg(feature = "native")]
use std::collections::HashMap;
use std::ops::DerefMut;
#[cfg(feature = "native")]
use std::sync::{OnceLock, RwLock};

use crate::db::EvmDb;
use crate::error::into_rpc_error;
use crate::evm::executor;
use crate::evm::primitive_types::{Receipt, TransactionSigned, TxSignedAndRecovered};
use crate::executor::get_cfg_env;
use crate::helpers::{from_recovered_with_block_context, prepare_call_env};
use crate::primitive_types::parse_synthetic_block_hash;
pub use crate::primitive_types::MaybeSealedBlock;
use crate::primitive_types::{synthetic_block_hash_for, SyntheticBlockWithoutRootsAndBloom};
use crate::{verify_contract_creation_allowlist, Evm, SealedBlock};
use alloy_consensus::{transaction::Recovered, Transaction as TransactionTrait, TxReceipt};
use alloy_consensus::{BlockHeader, EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Address, BlockHash, BlockNumber, Bloom, B64};
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
use sov_modules_api::{AccessoryStateReader, Amount, ApiStateAccessor, Spec};
use sov_rollup_interface::common::RollupHeight;
use sov_rpc_eth_types::{EthApiError, LogWithExecutionTimestamp, RpcInvalidTransactionError};

// Prune synthetic blocks more than this number of blocks away from the latest block.
const SYNTHETIC_BLOCKS_CACHE_PRUNE_INTERVAL: u64 = 20;

/// Cache of synthetic block states by hash.
///
/// Currently, there's no way to recreate the state of a synthetic block once it's gone. This is because
/// EVM state is shared with the sov-modules system, so any sov txs can impact the state of the synthetic block - but only
/// EVM tx bodies are stored in the EVM module. This cache saves a copy of the API state accessor for each synthetic block we make,
/// pruning them after some interval.
#[cfg(feature = "native")]
pub(crate) static SAVED_SYNTHETIC_BLOCK_STATE_BY_HASH: OnceLock<RwLock<SyntheticBlocksCache>> =
    OnceLock::new();

#[derive(Default)]
pub(crate) struct SyntheticBlocksCache {
    // Store ApiStateAccessor<S> by hash. We have to type erase because static variables can't store generic types.
    state_by_hash: HashMap<B256, Box<dyn Any + Send + Sync>>,
    // To allow for pruning, save a list of all synthetic block hashes for each block number
    // When those blocks are old enough, we prune them all.
    hashes_by_block_number: HashMap<u64, Vec<B256>>,
    oldest_block_number: u64,
}
impl SyntheticBlocksCache {
    fn insert_if_absent<S: Spec>(
        &mut self,
        block: &SyntheticBlockWithoutRootsAndBloom,
        state: &mut ApiStateAccessor<S>,
    ) {
        if let Entry::Vacant(entry) = self.state_by_hash.entry(block.hash()) {
            let state = state.clone_without_local_writes();
            let state: Box<dyn Any + Send + Sync> = Box::new(state);
            entry.insert(state);
            self.hashes_by_block_number
                .entry(block.block_number())
                .or_default()
                .push(block.hash());
            if block.block_number()
                > self.oldest_block_number + SYNTHETIC_BLOCKS_CACHE_PRUNE_INTERVAL
            {
                self.prune(block.block_number());
            }
        }
    }

    fn get_state_by_hash<S: Spec>(&self, hash: B256) -> Option<&ApiStateAccessor<S>> {
        self.state_by_hash.get(&hash).map(|b| b.downcast_ref::<ApiStateAccessor<S>>().expect("Attempted to get the wrong type out of the SyntheticBlocksCache. This is impossible unless you request a different Spec than you put in. It's a bug, please report it."))
    }
    fn prune(&mut self, block_number: u64) {
        let stop_at = block_number.saturating_sub(SYNTHETIC_BLOCKS_CACHE_PRUNE_INTERVAL);
        for number in self.oldest_block_number..stop_at {
            let hashes = self
                .hashes_by_block_number
                .remove(&number)
                .unwrap_or_default();
            for hash in hashes {
                self.state_by_hash.remove(&hash);
            }
        }
        self.oldest_block_number = stop_at;
    }
}

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
    /// A synthetic block which is not the current pending block.
    PastSynthetic {
        #[allow(missing_docs)]
        block_number: u64,
        #[allow(missing_docs)]
        last_tx_idx: u32,
    },
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
                        let tx_rpc = self.build_tx_with_maybe_effective_gas_price(
                            tx,
                            Some(block.header.hash()),
                            block.number,
                            block.header.base_fee_per_gas,
                            pos,
                            idx,
                            state,
                        );
                        Ok::<_, EthApiError>(tx_rpc)
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

    /// Gets a block requested by hash or number by an external caller.
    fn get_maybe_synthetic_block_for_rpc(
        &self,
        block_id: Option<BlockId>,
        kind: BlockTransactionsKind,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<Option<Block>, EthApiError> {
        let block_id = block_id.unwrap_or_else(BlockId::latest);
        let Some(block) = self.get_maybe_sealed_block_by_id(block_id, state)? else {
            return Ok(None);
        };
        match block {
            MaybeSealedBlock::Sealed(sealed) => {
                let block_size = sealed.rlp_size;
                let transactions = self.get_block_transactions(&sealed, kind, state)?;
                let header =
                    Header::from_consensus(sealed.header, None, Some(U256::from(block_size)));
                Ok(Some(Block {
                    header,
                    transactions,
                    uncles: vec![],
                    withdrawals: None,
                }))
            }
            // For pending blocks, we would like to avoid fetching the whole block body for performance reasons
            MaybeSealedBlock::PendingSynthetic(block) | MaybeSealedBlock::PastSynthetic(block) => {
                let tx_range_start = block.transactions.start;
                let (header, txs) = self.get_synthetic_block_contents_slow(block, state)?;
                let txs = match kind {
                    BlockTransactionsKind::Full => BlockTransactions::Full(
                        txs.into_iter()
                            .enumerate()
                            .map(|(pos, tx)| {
                                let tx_idx = tx_range_start + pos as u64;
                                self.build_tx_with_maybe_effective_gas_price(
                                    tx,
                                    Some(header.hash),
                                    header.number,
                                    header.base_fee_per_gas,
                                    pos,
                                    tx_idx,
                                    state,
                                )
                            })
                            .collect(),
                    ),
                    BlockTransactionsKind::Hashes => BlockTransactions::Hashes(
                        txs.into_iter()
                            .map(|tx| *tx.signed_transaction.hash())
                            .collect(),
                    ),
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

    fn build_tx_with_maybe_effective_gas_price<Accessor: AccessoryStateReader>(
        &self,
        tx: TxSignedAndRecovered,
        block_hash: Option<BlockHash>,
        block_number: BlockNumber,
        base_fee_per_gas: Option<u64>,
        pos: usize,
        tx_idx: u64,
        state: &mut Accessor,
    ) -> Transaction {
        let mut tx_rpc = from_recovered_with_block_context(
            tx.into(),
            block_hash,
            block_number,
            pos as u64,
            base_fee_per_gas,
        );
        let fee_paid = self.receipt_fee(tx_idx, state);
        if let Some((receipt, _)) = self.receipt(tx_idx, state) {
            if let Some(actual_effective_gas_price) =
                maybe_actual_effective_gas_price(block_number, receipt.gas_used, fee_paid)
            {
                tx_rpc.effective_gas_price = Some(actual_effective_gas_price);
            }
        }
        tx_rpc
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
        let header = Header::from_consensus(
            synthetic_block.header,
            None,
            Some(U256::from(synthetic_block.rlp_size)),
        );
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
        let tx_idx = self.tx_index(&hash, state)?;
        let tx = self.transaction(tx_idx, state)?;
        let block = self.get_maybe_sealed_block(tx.block_number, state)?;
        let pos = tx_idx - block.transactions_start();
        let tx = self.build_tx_with_maybe_effective_gas_price(
            tx,
            block.hash(),
            block.number(),
            block.maybe_partial_header().base_fee_per_gas,
            pos as usize,
            tx_idx,
            state,
        );
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
        let fee_paid = self.receipt_fee(number, state);
        Some(build_rpc_receipt(
            &block, tx, number, receipt, time, fee_paid,
        ))
    }

    fn get_receipt_by_index_in_block(
        &self,
        number: u64,
        block: &MaybeSealedBlock,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>> {
        let tx = self.transaction(number, state)?;
        let (receipt, time) = self.receipt(number, state)?;
        let fee_paid = self.receipt_fee(number, state);
        Some(build_rpc_receipt(
            block, tx, number, receipt, time, fee_paid,
        ))
    }

    fn get_receipts(
        &self,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<
        Option<Vec<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>>>,
        EthApiError,
    > {
        let block_id = block_id.unwrap_or_else(BlockId::latest);
        let Some(block) = self.get_maybe_sealed_block_by_id(block_id, state)? else {
            return Ok(None);
        };
        let mut receipts =
            Vec::with_capacity((block.transactions_end() - block.transactions_start()) as usize);
        for index in block.tx_range() {
            let Some(receipt) = self.get_receipt_by_index_in_block(index, &block, state) else {
                return Ok(None);
            };
            receipts.push(receipt);
        }
        Ok(Some(receipts))
    }

    fn call(
        &self,
        request: TransactionRequest,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<ResultAndState, EthApiError> {
        let block_env = self.resolve_block_env_for_call(block_id, state)?;
        let tx_env = prepare_call_env(&block_env, request.clone())?;
        let caller = tx_env.caller;
        let cfg = self.cfg_infallible(state);
        let cfg_env = get_cfg_env(&block_env, &cfg, Some(get_cfg_env_template()));
        let mut maybe_archival_state = self.resolve_state_for_block_id(block_id, state)?;
        let mut evm_db: EvmDb<_, S> = self.db(maybe_archival_state.deref_mut());
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
            let Some(pending) = self.pending_block(Some(block_env), state) else {
                return Some(MaybeSealedBlock::Sealed(self.latest_block(state)));
            };
            return Some(MaybeSealedBlock::PendingSynthetic(pending));
        }

        None
    }

    fn finalized_block_number(&self, state: &mut ApiStateAccessor<S>) -> u64 {
        let mut archival = state
            .build_finalized_state()
            .expect("Bug: wrong slot number has been passed");
        *self.block_numbers(&mut archival).end()
    }

    fn block_tag_to_pending_or_block(
        &self,
        block: BlockNumberOrTag,
        state: &mut ApiStateAccessor<S>,
    ) -> PendingOrBlock {
        match block {
            BlockNumberOrTag::Earliest => {
                let block_numbers = self.block_numbers(state);
                PendingOrBlock::Number(*block_numbers.start())
            }
            // We treat latest and pending the same to avoid foundry issues
            BlockNumberOrTag::Latest | BlockNumberOrTag::Pending => PendingOrBlock::Pending,
            BlockNumberOrTag::Finalized | BlockNumberOrTag::Safe => {
                PendingOrBlock::Number(self.finalized_block_number(state))
            }
            BlockNumberOrTag::Number(number) => PendingOrBlock::Number(number),
        }
    }

    fn block_id_to_pending_or_block(
        &self,
        block_id: BlockId,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<PendingOrBlock, EthApiError> {
        Ok(match block_id {
            BlockId::Number(tag) => self.block_tag_to_pending_or_block(tag, state),
            BlockId::Hash(hash) => {
                if let Some((block_number, last_tx_idx)) = parse_synthetic_block_hash(&hash.into())
                {
                    return Ok(PendingOrBlock::PastSynthetic {
                        block_number,
                        last_tx_idx,
                    });
                }
                let Some(number) = self
                    .block_hash_to_number
                    .get(&hash.block_hash, state)
                    .unwrap_infallible()
                else {
                    return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash)));
                };
                PendingOrBlock::Number(number)
            }
        })
    }

    /// Converts BlockNumberOrTag into number.
    /// Can panic if passed ApiStateAccessor has been constructed with wrong finalized_slot_height.
    pub fn resolve_block_number(
        &self,
        block: BlockNumberOrTag,
        state: &mut ApiStateAccessor<S>,
    ) -> BlockNumber {
        let block_numbers = self.block_numbers(state);
        let block_number = match block {
            BlockNumberOrTag::Earliest => *block_numbers.start(),
            BlockNumberOrTag::Finalized | BlockNumberOrTag::Safe => {
                self.finalized_block_number(state)
            }
            BlockNumberOrTag::Number(nr) => nr,
            // We treat latest and pending the same to avoid foundry issues
            BlockNumberOrTag::Latest | BlockNumberOrTag::Pending => {
                if self.has_pending_block(state) {
                    *block_numbers.end() + 1
                } else {
                    *block_numbers.end()
                }
            }
        };
        block_number
    }

    /// Retrieve a block by its id.
    /// Can panic if passed ApiStateAccessor has been constructed with wrong finalized_slot_height.
    pub fn get_maybe_sealed_block_by_id(
        &self,
        block_id: BlockId,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<Option<MaybeSealedBlock>, EthApiError> {
        Ok(match block_id {
            BlockId::Number(tag) => {
                let pending_or_block = self.block_tag_to_pending_or_block(tag, state);
                match pending_or_block {
                    PendingOrBlock::Number(number) => self.get_maybe_sealed_block(number, state),
                    PendingOrBlock::Pending => match self.pending_block(None, state) {
                        Some(pending) => Some(MaybeSealedBlock::PendingSynthetic(pending)),
                        None => {
                            // pending_block() returns None when there are no pending txs, and so fall back to the latest sealed block in those cases
                            return Ok(Some(MaybeSealedBlock::Sealed(self.latest_block(state))));
                        }
                    },
                    PendingOrBlock::PastSynthetic { .. } => {
                        unreachable!()
                    }
                }
            }
            BlockId::Hash(hash) => {
                // If the hash is synthetic, handle that special case
                if let Some((block_number, num_txs)) = parse_synthetic_block_hash(&hash.into()) {
                    // Prerequisite: Check that the synthetic block exists and is recent enough to still be saved.
                    // This ensures that any calls to "getBlockByHash" will succeed only if "eth_call" would succeed given the same hash.
                    {
                        let Some(cache) = SAVED_SYNTHETIC_BLOCK_STATE_BY_HASH.get() else {
                            return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash)));
                        };
                        let lock = cache
                            .read()
                            .expect("Failed to read from synthetic blocks cache");
                        if lock.get_state_by_hash::<S>(hash.into()).is_none() {
                            return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash)));
                        }
                    }

                    // Case 1: The synthetic block has the same block number as the pending block; that's a harder special case where we need to populate its fields from the pending block
                    let block_env = self.block_env(state).unwrap_infallible();
                    if block_env.number == block_number {
                        let Some(mut newest_pending_block) =
                            self.pending_block(Some(block_env), state)
                        else {
                            return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash)));
                        };
                        // Check that the number of txs requested is no more than the number of txs in the pending block. If not, this block doesn't exist - return early
                        if num_txs as u64 > newest_pending_block.num_transactions() {
                            return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash)));
                        }
                        // Check if the number of txs requested is exactly the same as the number of txs in the pending block. If so, this is the pending block! return it
                        if num_txs as u64 == newest_pending_block.num_transactions() {
                            return Ok(Some(MaybeSealedBlock::PendingSynthetic(
                                newest_pending_block,
                            )));
                        }
                        // Otherwise, this is at the same as the pending block but with fewer txs - truncate the pending block to the number of txs requested
                        newest_pending_block.transactions.end =
                            newest_pending_block.transactions.start + num_txs as u64;
                        return Ok(Some(MaybeSealedBlock::PendingSynthetic(
                            newest_pending_block,
                        )));
                    }

                    // Case 2: The synthetic block is not pending. We can just populate the fields from the sealed block at that height if one exists
                    return Ok(Some(MaybeSealedBlock::PastSynthetic(
                        self.past_synthetic_block(block_number, num_txs, state)?,
                    )));
                }
                self.block_hash_to_number
                    .get(&hash.block_hash, state)
                    .unwrap_infallible()
                    .and_then(|number| self.get_maybe_sealed_block(number, state))
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
    /// latest_sealed_block.number + 1. (Both are updated during the finalize_hook, so they're atomic). Returns None if there are no txs in the pending block.
    /// In that case, we should use the latest sealed block instead.
    ///
    /// Note that values in the block_env (including the block number there!) are updated during the begin_rollup_block_hook, so the values here may be stale
    /// if this function is called while no rollup block is in progress. In that case, the number of transactions will be zero.
    ///
    /// This function also caches the API state at the time this pending block was created in `SYNTHETIC_BLOCKS_CACHE_BY_HASH`. This allows recreate the state
    /// as of the synthetic block as long as it remains in cache, which would otherwise be impossible. See the docs on `SYNTHETIC_BLOCKS_CACHE_BY_HASH` for more details
    pub fn pending_block(
        &self,
        block_env: Option<BlockEnv>,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<SyntheticBlockWithoutRootsAndBloom> {
        self.maybe_pending_block(false, block_env, state)
    }

    // Populates a synthetic block from a sealed block.
    fn past_synthetic_block(
        &self,
        block_number: u64,
        num_txs: u32,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<SyntheticBlockWithoutRootsAndBloom, EthApiError> {
        // Populate the header from the sealed block
        let Some(block) = self.blocks.get(&block_number, state).unwrap_infallible() else {
            return Err(EthApiError::HeaderNotFound(BlockId::Hash(
                synthetic_block_hash_for(block_number, num_txs).into(),
            )));
        };

        if block.transactions().end - block.transactions().start < num_txs as u64 {
            return Err(EthApiError::HeaderNotFound(BlockId::Hash(
                synthetic_block_hash_for(block_number, num_txs).into(),
            )));
        }
        let first_tx_index = block.transactions().start;
        let last_tx_index = first_tx_index
            .checked_add(num_txs as u64)
            .expect("Number of transactions should fit in u64");

        let header = alloy_consensus::Header {
            parent_hash: block.header.parent_hash(),
            number: block_number,
            timestamp: block.header.timestamp,
            excess_blob_gas: block.header.excess_blob_gas,
            base_fee_per_gas: block.header.base_fee_per_gas,
            gas_limit: block.header.gas_limit,

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

        let block = SyntheticBlockWithoutRootsAndBloom::new(header, first_tx_index..last_tx_index);

        Ok(block)
    }

    fn maybe_pending_block(
        &self,
        allow_empty: bool,
        block_env: Option<BlockEnv>,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<SyntheticBlockWithoutRootsAndBloom> {
        let block_numbers = self.block_numbers(state);

        let head_block = self
            .blocks
            .get(block_numbers.end(), state)
            .unwrap_infallible()
            .expect("Block should exist as index is inside block_numbers");
        let current_block_env =
            block_env.unwrap_or_else(|| self.block_env(state).unwrap_infallible());

        assert_eq!(&head_block.header.number, block_numbers.end());

        let pending_transactions_len = self.pending_transactions.len(state).unwrap_infallible();
        if pending_transactions_len == 0 && !allow_empty {
            return None;
        }

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

        let block = SyntheticBlockWithoutRootsAndBloom::new(header, start..end);
        #[cfg(feature = "native")]
        {
            let state_by_hash =
                SAVED_SYNTHETIC_BLOCK_STATE_BY_HASH.get_or_init(|| RwLock::new(Default::default()));
            // TODO: Optimization, lower priority - move the rwlock inside the cache and "read" to check if the block is already in the cache before grabbing the write lock.
            state_by_hash
                .write()
                .expect("Failed to write to synthetic blocks cache")
                .insert_if_absent(&block, state);
        }

        Some(block)
    }

    /// Returns the header of the newest synthetic block, even if it's empty.
    pub fn get_newest_synthetic_header(
        &self,
        state: &mut ApiStateAccessor<S>,
    ) -> SyntheticBlockWithoutRootsAndBloom {
        self.maybe_pending_block(true, None, state)
            .expect("Maybe pending block should never return None if allow_empty is true")
    }

    /// Resolves a block ID to either current or archival state.
    ///
    /// Explicit numeric selectors are resolved by block kind, not by height arithmetic:
    /// - if the number identifies the current synthetic pending block, use `Current`
    /// - otherwise resolve through archival state for that exact block number.
    ///
    /// This avoids coupling correctness to `rollup_height` transition timing.
    fn resolve_state_for_block_id<'a>(
        &self,
        block_id: Option<BlockId>,
        state: &'a mut ApiStateAccessor<S>,
    ) -> Result<MaybeArchivalState<'a, S>, EthApiError> {
        let block_id = block_id.unwrap_or_else(BlockId::latest);
        let pending_or_block_nr = self.block_id_to_pending_or_block(block_id, state)?;
        match pending_or_block_nr {
            PendingOrBlock::Pending => Ok(MaybeArchivalState::Current(state)),
            PendingOrBlock::Number(number) => {
                // Always prefer archival for explicitly sealed block numbers.
                if self.blocks.get(&number, state).unwrap_infallible().is_none()
                    // Treat explicit pending-block numbers as current only when there are pending txs.
                    && self.has_pending_block(state)
                    && self.block_env(state).unwrap_infallible().number == number
                {
                    return Ok(MaybeArchivalState::Current(state));
                }
                let archival_state = state.get_archival_state(RollupHeight::new(number))?;
                Ok(MaybeArchivalState::Archival(archival_state.into()))
            }
            PendingOrBlock::PastSynthetic {
                block_number,
                last_tx_idx,
            } => {
                let hash = synthetic_block_hash_for(block_number, last_tx_idx);
                let Some(cache) = SAVED_SYNTHETIC_BLOCK_STATE_BY_HASH.get() else {
                    return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash.into())));
                };
                let lock = cache
                    .read()
                    .expect("Failed to read from synthetic blocks cache");
                let Some(state) = lock
                    .get_state_by_hash(hash)
                    .map(|state| state.clone_without_local_writes())
                else {
                    return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash.into())));
                };
                Ok(MaybeArchivalState::Synthetic(Box::new(state)))
            }
        }
    }

    fn resolve_block_env_for_call(
        &self,
        block_id: Option<BlockId>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<BlockEnv, EthApiError> {
        let block_id = block_id.unwrap_or_else(BlockId::latest);
        let maybe_block = self
            .get_maybe_sealed_block_by_id(block_id, state)?
            .ok_or(EthApiError::UnknownBlock)?;

        let block_env = match maybe_block {
            MaybeSealedBlock::PendingSynthetic(_) => self.block_env(state).unwrap_infallible(),
            MaybeSealedBlock::Sealed(sealed_block) => BlockEnv::from(sealed_block),
            MaybeSealedBlock::PastSynthetic(synthetic_block) => BlockEnv::from(synthetic_block),
        };
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

fn maybe_actual_effective_gas_price(
    block_number: u64,
    gas_used: u64,
    fee_paid: Option<Amount>,
) -> Option<u128> {
    let apply_actual_fee_after_height: u64 = config_value!("EVM_RECEIPT_ACTUAL_FEE_HEIGHT");
    if block_number <= apply_actual_fee_after_height || gas_used == 0 {
        return None;
    }

    fee_paid.map(|fee_paid| fee_paid.0 / u128::from(gas_used))
}

// modified from: https://github.com/paradigmxyz/reth many times
pub(crate) fn build_rpc_receipt(
    block: &MaybeSealedBlock,
    tx: TxSignedAndRecovered,
    tx_number: u64,
    receipt: Receipt,
    time: Time,
    fee_paid: Option<Amount>,
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

    // EIP-1559 effective gas price calculation (https://github.com/ethereum/EIPs/blob/0d31c18725202ae8bbfb82b8d3d028ad1810d360/EIPS/eip-1559.md?plain=1#L222-L224):
    //   priority_fee_per_gas = min(transaction.max_priority_fee_per_gas,
    //                              transaction.max_fee_per_gas - block.base_fee_per_gas)
    //   effective_gas_price = priority_fee_per_gas + block.base_fee_per_gas
    let eip_1559_effective_gas_price = transaction
        .inner()
        .effective_gas_price(block.maybe_partial_header().base_fee_per_gas);

    // Once activated, prefer the fee paid in the Sovereign gas meter when available.
    // This keeps receipt fee semantics aligned with balance deltas.
    let effective_gas_price =
        maybe_actual_effective_gas_price(block.number(), receipt.gas_used, fee_paid)
            // Keep compatibility for historical data where metadata may be missing.
            .unwrap_or(eip_1559_effective_gas_price);

    TransactionReceipt {
        inner: ReceiptEnvelope::Eip1559(ReceiptWithBloom::new(rpc_receipt, logs_bloom)),
        transaction_hash,
        transaction_index: Some(transaction_index),
        block_hash,
        block_number,
        gas_used: receipt.gas_used,
        effective_gas_price,
        blob_gas_used: None,
        blob_gas_price: None,
        from,
        to,
        contract_address,
    }
}
