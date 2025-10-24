use crate::conversions::create_block_env;
use crate::evm::primitive_types::{Block, TransactionSigned};
use crate::{Evm, PendingTransaction};
use alloy_consensus::proofs::{calculate_receipt_root, calculate_transaction_root};
use alloy_consensus::{TxReceipt, EMPTY_OMMER_ROOT_HASH};
use alloy_primitives::{Bloom, Bytes, B256, B64, U256};
use revm::primitives::constants::BLOCK_HASH_HISTORY;
#[cfg(feature = "native")]
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
#[cfg(feature = "native")]
use sov_modules_api::{AccessoryStateReaderAndWriter, FinalizeHook};
use sov_modules_api::{BlockHooks, Spec, StateCheckpoint};
use sov_state::{ProvableNamespace, StateRoot, Storage};
#[cfg(feature = "native")]
use std::convert::Infallible;
#[cfg(feature = "native")]
use std::ops::RangeInclusive;

impl<S: Spec> BlockHooks for Evm<S> {
    type Spec = S;
    /// Logic executed at the beginning of the slot. Here we set the root hash of the previous head.
    fn begin_rollup_block_hook(
        &mut self,
        pre_state_user_root: &<S::Storage as Storage>::Root,
        state: &mut StateCheckpoint<S>,
    ) {
        let mut parent_block = self.head(state);

        let pre_state_user_root: [u8; 32] =
            pre_state_user_root.namespace_root(ProvableNamespace::User);

        // Here we set the parent's state root to the previous state root
        parent_block.header.state_root =
            // We have to force the conversion to [u8;32] to prevent the `from_slice` method from panicking
            B256::from_slice(&pre_state_user_root);
        self.head.set(&parent_block, state).unwrap_infallible();
        let parent_header = &parent_block.header;
        self.block_hashes
            .set(&(parent_header.number), &parent_header.hash_slow(), state)
            .unwrap_infallible();
        if parent_header.number >= BLOCK_HASH_HISTORY {
            self.block_hashes
                .delete(&(parent_header.number - BLOCK_HASH_HISTORY), state)
                .unwrap_infallible();
        }

        let cfg = self.cfg_infallible(state);

        let new_block_number = parent_block
            .header
            .number
            .checked_add(1)
            .expect("We should never have more than u64::MAX blocks");

        let new_timestamp = self
            .chain_state_module
            .get_time(state)
            .unwrap_infallible()
            .as_millis() as u64;

        let new_pending_env = create_block_env(
            self.base_fee(),
            cfg.chain_spec.block_gas_limit,
            new_timestamp,
            cfg.chain_spec.coinbase,
            new_block_number,
            Some(B256::from(pre_state_user_root)),
        );

        self.block_env
            .set(&new_pending_env, state)
            .unwrap_infallible();
    }

    /// Logic executed at the end of the slot. Here, we generate an authenticated block and set it as the new head of the chain.
    /// It's important to note that the state root hash is not known at this moment, so we postpone setting this field until the begin_rollup_block_hook of the next slot.
    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<S>) {
        let block_env = self.block_env(state).unwrap_infallible();
        let parent_block = self.head(state).seal();

        let expected_block_number = parent_block.header.number.wrapping_add(1);
        assert_eq!(
            block_env.number, expected_block_number,
            "Pending head must be set to block {}, but found block {}",
            expected_block_number, block_env.number
        );

        let pending_transactions: Vec<PendingTransaction> =
            self.pending_transactions.collect_infallible(state);

        self.pending_transactions.clear(state).unwrap_infallible();

        let start_tx_index = parent_block.transactions.end;

        let gas_used = pending_transactions
            .last()
            .map_or(0u64, |tx| tx.receipt.receipt.cumulative_gas_used);

        let transactions: Vec<TransactionSigned> = pending_transactions
            .iter()
            .map(|tx| tx.transaction.signed_transaction.clone())
            .collect();

        let receipts: Vec<reth_primitives::ReceiptWithBloom<&reth_primitives::Receipt>> =
            pending_transactions
                .iter()
                .map(|tx| tx.receipt.receipt.with_bloom_ref())
                .collect();
        let receipts_root = calculate_receipt_root(receipts.as_slice());
        let transactions_root = calculate_transaction_root(transactions.as_slice());

        let header = alloy_consensus::Header {
            timestamp: block_env.timestamp.to::<u64>(),
            parent_hash: parent_block.header.seal(),
            number: block_env.number.to::<u64>(),
            beneficiary: parent_block.header.beneficiary,
            // This will be set in finalize_hook or in the next begin_rollup_block_hook
            state_root: Default::default(),
            transactions_root,
            receipts_root,
            logs_bloom: receipts
                .iter()
                .fold(Bloom::ZERO, |bloom, r| bloom | r.bloom()),
            gas_limit: block_env.gas_limit,
            gas_used,
            mix_hash: block_env.prevrandao.map_or(B256::ZERO, B256::from),
            excess_blob_gas: block_env
                .blob_excess_gas_and_price
                .map(|blob_gas| blob_gas.excess_blob_gas),
            base_fee_per_gas: Some(block_env.basefee),
            // Default values
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            difficulty: U256::ZERO,
            extra_data: Bytes::default(),
            nonce: B64::ZERO,
            withdrawals_root: None,
            blob_gas_used: None,
            parent_beacon_block_root: None,
            requests_hash: None,
        };

        let end_tx_index = start_tx_index
            .checked_add(pending_transactions.len() as u64)
            .expect("We should never have more than u64::MAX transactions");

        let block = Block {
            header,
            transactions: start_tx_index..end_tx_index,
        };

        self.head.set(&block, state).unwrap_infallible();

        #[cfg(feature = "native")]
        {
            let mut accessory_state = state.accessory_state();
            self.pending_head
                .set(&block, &mut accessory_state)
                .unwrap_infallible();
        }

        self.pending_transactions.clear(state).unwrap_infallible();
    }
}

#[cfg(feature = "native")]
impl<S: Spec> FinalizeHook for Evm<S> {
    type Spec = S;

    /// This logic is executed after calculating the root hash.
    /// At this point, it is impossible to alter state variables because the state root is fixed.
    /// However, non-state data can be modified.
    /// This function's purpose is to add the block to the (non-authenticated) blocks structure,
    /// enabling block-related RPC queries.
    fn finalize_hook(
        &mut self,
        root_hash: &<S::Storage as Storage>::Root,
        state: &mut impl AccessoryStateReaderAndWriter,
    ) {
        let mut block = self
            .pending_head
            .get(state)
            .unwrap_infallible()
            .expect("We set `pending_head` in `end_rollup_block_hook`");

        let user_space_root_hash: [u8; 32] = root_hash.namespace_root(ProvableNamespace::User);
        block.header.state_root = user_space_root_hash.into();

        let block_number = block.header.number;
        let sealed_block = block.seal();

        let block_numbers_range = self.block_numbers(state);

        let new_block_numbers_range =
            crate_range_from(block_numbers_range, None, Some(block_number));

        self.block_numbers
            .set(&new_block_numbers_range, state)
            .unwrap_infallible();

        self.blocks
            .set(&sealed_block.header.number, &sealed_block, state)
            .unwrap_infallible();

        self.block_hash_to_number
            .set(
                &sealed_block.header.seal(),
                &sealed_block.header.number,
                state,
            )
            .unwrap_infallible();
        self.pending_head.delete(state).unwrap_infallible();

        self.prune(state).unwrap_infallible();
    }
}

#[cfg(feature = "native")]
impl<S: Spec> Evm<S> {
    fn prune(&mut self, state: &mut impl AccessoryStateReaderAndWriter) -> Result<(), Infallible> {
        let block_pruning_threshold = config_value!("EVM_BLOCK_PRUNING_THRESHOLD");
        let block_numbers = self.block_numbers(state);

        if let Some(last_to_remove) = block_numbers.end().checked_sub(block_pruning_threshold) {
            let block_numbers_to_remove =
                crate_range_from(block_numbers.clone(), None, Some(last_to_remove));

            for block_number in block_numbers_to_remove {
                self.prune_block(block_number, state).expect(
                    "Prunning block should succeed as block_numbers indicate - block exists",
                );
            }

            let new_block_numbers = crate_range_from(block_numbers, Some(last_to_remove + 1), None);
            assert!(new_block_numbers.end() - new_block_numbers.start() < block_pruning_threshold);
            self.block_numbers.set(&new_block_numbers, state)?;
        }

        Ok(())
    }

    fn prune_block(
        &mut self,
        number: u64,
        state: &mut impl AccessoryStateReaderAndWriter,
    ) -> Option<()> {
        let block = self.blocks.remove(&number, state).unwrap_infallible()?;
        let hash = block.header.hash();
        self.block_hash_to_number
            .remove(&hash, state)
            .unwrap_infallible()?;

        for tx_idx in block.transactions {
            self.prune_tx(tx_idx, state)
                .expect("Cascade tx delete should succeed as each tx belongs to a single block");
        }
        Some(())
    }

    fn prune_tx(&mut self, idx: u64, state: &mut impl AccessoryStateReaderAndWriter) -> Option<()> {
        let transaction = self.transactions.remove(&idx, state).unwrap_infallible()?;
        let tx_hash = transaction.signed_transaction.hash();
        self.transaction_hashes
            .remove(tx_hash, state)
            .unwrap_infallible()?;
        self.receipts.remove(&idx, state).unwrap_infallible()?;
        Some(())
    }
}

#[cfg(feature = "native")]
fn crate_range_from(
    mut range: RangeInclusive<u64>,
    new_start: Option<u64>,
    new_end: Option<u64>,
) -> RangeInclusive<u64> {
    if let Some(new_start) = new_start {
        range = RangeInclusive::new(new_start, *range.end());
    }
    if let Some(new_end) = new_end {
        range = RangeInclusive::new(*range.start(), new_end);
    }
    range
}
