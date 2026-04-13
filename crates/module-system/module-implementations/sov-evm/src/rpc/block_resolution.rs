use std::ops::DerefMut;
use std::sync::RwLock;

use alloy_consensus::BlockHeader;
use alloy_consensus::{EMPTY_OMMER_ROOT_HASH, EMPTY_ROOT_HASH};
use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Address, BlockHash, BlockNumber, Bloom, Bytes, B256, B64, U256};
use alloy_rpc_types::{
    Block, BlockTransactions, BlockTransactionsKind, Header, ReceiptEnvelope, Transaction,
    TransactionReceipt,
};
use revm::context::BlockEnv;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{AccessoryStateReader, ApiStateAccessor, Spec};
use sov_rollup_interface::common::RollupHeight;
use sov_rpc_eth_types::{EthApiError, LogWithExecutionTimestamp};

use super::maybe_archival_state::MaybeArchivalState;
use super::receipt_builder::{build_rpc_receipt, maybe_actual_effective_gas_price};
use super::synthetic_block_cache::SAVED_SYNTHETIC_BLOCK_STATE_BY_HASH;
use crate::evm::primitive_types::TxSignedAndRecovered;
use crate::helpers::from_recovered_with_block_context;
use crate::primitive_types::{
    parse_synthetic_block_hash, synthetic_block_hash_for, MaybeSealedBlock,
    SyntheticBlockWithoutRootsAndBloom,
};
use crate::{Evm, SealedBlock};

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
    pub(crate) fn get_maybe_synthetic_block_for_rpc(
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

    pub(crate) fn build_tx_with_maybe_effective_gas_price<Accessor: AccessoryStateReader>(
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
            if let Some(actual_effective_gas_price) = maybe_actual_effective_gas_price(
                block_number,
                receipt.gas_used,
                fee_paid,
                base_fee_per_gas,
            ) {
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

    pub(crate) fn get_contract_code(
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
        // `bytes()` on analyzed legacy bytecode includes the appended STOP byte used for
        // execution; RPC must return the original deployed bytecode.
        Some(code.original_bytes())
    }

    pub(crate) fn get_transaction(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<Transaction> {
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

    pub(crate) fn get_receipt_by_hash(
        &self,
        hash: B256,
        state: &mut ApiStateAccessor<S>,
    ) -> Option<TransactionReceipt<ReceiptEnvelope<LogWithExecutionTimestamp>>> {
        let number = self.tx_index(&hash, state)?;
        self.get_receipt_by_index(number, state)
    }

    pub(crate) fn get_receipt_by_index(
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

    pub(crate) fn get_receipt_by_index_in_block(
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

    pub(crate) fn get_receipts(
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

    pub(crate) fn block_tag_to_pending_or_block(
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

    pub(crate) fn block_id_to_pending_or_block(
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

                    // Case 1: The synthetic block has the same block number as the pending block;
                    // that's a harder special case where we need to populate its fields from
                    // the pending block.
                    let block_env = self.block_env(state).unwrap_infallible();
                    if block_env.number == block_number {
                        let Some(mut newest_pending_block) =
                            self.pending_block(Some(block_env), state)
                        else {
                            return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash)));
                        };
                        // Check that the number of txs requested is no more than the number of
                        // txs in the pending block. If not, this block doesn't exist.
                        if num_txs as u64 > newest_pending_block.num_transactions() {
                            return Err(EthApiError::HeaderNotFound(BlockId::Hash(hash)));
                        }
                        // If number of txs matches exactly, this is the pending block.
                        if num_txs as u64 == newest_pending_block.num_transactions() {
                            return Ok(Some(MaybeSealedBlock::PendingSynthetic(
                                newest_pending_block,
                            )));
                        }
                        // Otherwise this is the pending block truncated to fewer transactions.
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

            // Values that never need to be initialized
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

            // Values that never need to be initialized
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

    /// Resolves a block ID to current, archival, or cached synthetic state.
    ///
    /// Explicit numeric selectors are resolved by block kind, not by height arithmetic:
    /// - if the number identifies the current synthetic pending block, use `Current`
    /// - otherwise resolve through archival state for that exact block number.
    ///
    /// This avoids coupling correctness to `rollup_height` transition timing.
    pub(crate) fn resolve_state_for_block_id<'a>(
        &self,
        block_id: Option<BlockId>,
        state: &'a mut ApiStateAccessor<S>,
    ) -> Result<MaybeArchivalState<'a, S>, EthApiError> {
        let block_id = block_id.unwrap_or_else(BlockId::latest);
        let pending_or_block_nr = self.block_id_to_pending_or_block(block_id, state)?;
        match pending_or_block_nr {
            PendingOrBlock::Pending => Ok(MaybeArchivalState::Current(state)),
            PendingOrBlock::Number(number) => {
                // The live pending block is addressable by explicit number before it
                // is sealed. `get_archival_state` may still succeed for the current
                // rollup height via uncommitted changes, so archival lookup success
                // is not a valid test for "this EVM block is sealed". Match `dev`:
                // explicit current pending block numbers must resolve to `Current`.
                if self
                    .blocks
                    .get(&number, state)
                    .unwrap_infallible()
                    .is_none()
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

    pub(crate) fn resolve_block_env_for_call(
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
