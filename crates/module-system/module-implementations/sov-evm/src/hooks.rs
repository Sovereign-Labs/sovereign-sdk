use crate::evm::primitive_types::{Block, TransactionSigned};
use crate::{BlockEnv, Evm, PendingTransaction};
use alloy_consensus::proofs::{calculate_receipt_root, calculate_transaction_root};
use alloy_consensus::TxReceipt;
use alloy_primitives::Bloom;
use alloy_primitives::{B256, U256};
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
        let mut parent_block = self
            .head
            .get(state)
            .unwrap_infallible()
            // This is justified. We set the head at genesis and never remove it — only overwrite it.
            .expect("The impossible happened: Head block is empty");

        let pre_state_user_root: [u8; 32] =
            pre_state_user_root.namespace_root(ProvableNamespace::User);

        // Here we set the parent's state root to the previous state root
        parent_block.header.state_root =
            // We have to force the conversion to [u8;32] to prevent the `from_slice` method from panicking
            B256::from_slice(&pre_state_user_root);
        self.head.set(&parent_block, state).unwrap_infallible();

        let cfg = self.cfg_infallible(state);

        let new_block_number = parent_block
            .header
            .number
            .checked_add(1)
            // This is justified. We will never have so many blocks.
            .expect("The impossible happened: Block number overflow");

        let new_timestamp = self
            .chain_state_module
            .get_time(state)
            .unwrap_infallible()
            .as_millis() as u64;

        let new_pending_env = BlockEnv {
            number: U256::from(new_block_number),
            beneficiary: cfg.chain_spec.coinbase,
            timestamp: U256::from(new_timestamp),
            // WARNING: `prevrandao`` value is predictable up to [`DEFERRED_SLOTS_COUNT`] in advance,
            // Users should follow the same best practice that they would on Ethereum and use future randomness.
            // See: https://eips.ethereum.org/EIPS/eip-4399#tips-for-application-developers
            prevrandao: Some(B256::from(pre_state_user_root)),
            gas_limit: cfg.chain_spec.block_gas_limit,
            ..Default::default()
        };
        self.block_env
            .set(&new_pending_env, state)
            .unwrap_infallible();
    }

    /// Logic executed at the end of the slot. Here, we generate an authenticated block and set it as the new head of the chain.
    /// It's important to note that the state root hash is not known at this moment, so we postpone setting this field until the begin_rollup_block_hook of the next slot.
    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<S>) {
        let block_env = self
            .block_env
            .get(state)
            .unwrap_infallible()
            // This is justified. We set `pending_head` in `end_rollup_block_hook`.
            .expect("The impossible happened: Pending block is empty");

        let parent_block = self
            .head
            .get(state)
            .unwrap_infallible()
            // This is justified. We set the head at genesis and never remove it — only overwrite it.
            .expect("The impossible happened: Head block is empty")
            .seal();

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
            ..Default::default()
        };

        let end_tx_index = start_tx_index
            .checked_add(pending_transactions.len() as u64)
            // This is justified. We will never have that many txs.
            .expect("The impossible happened: Tx count overflow");

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
            // Justified, we set `pending_head` in `end_rollup_block_hook`.
            .expect("The impossible happened: the pending block should always be set.");

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

        self.block_hashes
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
                let block = self
                    .blocks
                    .remove(&block_number, state)?
                    // Safe because we already checked block_numbers.len()
                    .expect("Impossible happened: no block available to prune");

                let block_hash = block.header.hash();
                self.block_hashes
                    .remove(&block_hash, state)?
                    // Safe, since we keep one block_hash per block.
                    .expect("Impossible happened: no block_hasha available to prune");

                for tx_idx in block.transactions {
                    let transaction = self
                        .transactions
                        .remove(&tx_idx, state)?
                        // Safe because we already checked transactions.len()
                        // Safe, since we keep one tx_hash per tx.
                        .expect("Impossible happened: no transaction_hashes available to prune");

                    let tx_hash = transaction.signed_transaction.hash();

                    self.transaction_hashes
                        .remove(tx_hash, state)?
                        // Safe, since we keep one tx_hash per tx.
                        .expect("Impossible happened: no transaction_hashes available to prune");

                    self.receipts
                        .remove(&tx_idx, state)?
                        // Safe because we already checked receipts.len()
                        .expect("Impossible happened: no receipts available to prune");
                }
            }

            let new_block_numbers = crate_range_from(block_numbers, Some(last_to_remove + 1), None);
            assert!(new_block_numbers.end() - new_block_numbers.start() < block_pruning_threshold);
            self.block_numbers.set(&new_block_numbers, state)?;
        }

        Ok(())
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
