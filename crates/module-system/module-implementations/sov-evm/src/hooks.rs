use crate::evm::primitive_types::Block;
use crate::{BlockEnv, Evm, PendingTransaction};
use alloy_consensus::constants::KECCAK_EMPTY;
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
            .expect("Head block should always be set");

        let pre_state_user_root: [u8; 32] =
            pre_state_user_root.namespace_root(ProvableNamespace::User);

        // Here we set the parent's state root to the previous state root
        parent_block.header.state_root =
            // We have to force the conversion to [u8;32] to prevent the `from_slice` method from panicking
            B256::from_slice(&pre_state_user_root);
        self.head.set(&parent_block, state).unwrap_infallible();

        let cfg = self.cfg_infallible(state);

        let new_pending_env = BlockEnv {
            number: U256::from(parent_block.header.number.wrapping_add(1)),
            beneficiary: cfg.chain_spec.coinbase,
            timestamp: U256::from(
                parent_block
                    .header
                    .timestamp
                    .saturating_add(cfg.chain_spec.block_timestamp_delta),
            ),
            // WARNING: `prevrandao`` value is predictable up to [`DEFERRED_SLOTS_COUNT`] in advance,
            // Users should follow the same best practice that they would on Ethereum and use future randomness.
            // See: https://eips.ethereum.org/EIPS/eip-4399#tips-for-application-developers
            prevrandao: Some(B256::from(pre_state_user_root)),
            basefee: parent_block
                .header
                .next_block_base_fee(cfg.chain_spec.base_fee_params)
                .unwrap(),
            gas_limit: cfg.chain_spec.block_gas_limit,
            difficulty: Default::default(),
            blob_excess_gas_and_price: None,
        };
        self.block_env
            .set(&new_pending_env, state)
            .unwrap_infallible();
    }

    /// Logic executed at the end of the slot. Here, we generate an authenticated block and set it as the new head of the chain.
    /// It's important to note that the state root hash is not known at this moment, so we postpone setting this field until the begin_rollup_block_hook of the next slot.
    fn end_rollup_block_hook(&mut self, state: &mut StateCheckpoint<S>) {
        let cfg = self.cfg_infallible(state);

        let block_env = self
            .block_env
            .get(state)
            .unwrap_infallible()
            .expect("Pending block should always be set");

        let parent_block = self
            .head
            .get(state)
            .unwrap_infallible()
            .expect("Head block should always be set")
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

        let transactions: Vec<reth_primitives::TransactionSigned> = pending_transactions
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
            parent_hash: parent_block.header.seal(),
            timestamp: block_env.timestamp.to::<u64>(),
            number: block_env.number.to::<u64>(),
            beneficiary: parent_block.header.beneficiary,
            // This will be set in finalize_hook or in the next begin_rollup_block_hook
            state_root: KECCAK_EMPTY,
            transactions_root,
            receipts_root,
            logs_bloom: receipts
                .iter()
                .fold(Bloom::ZERO, |bloom, r| bloom | r.bloom()),
            gas_limit: block_env.gas_limit,
            gas_used,
            mix_hash: block_env.prevrandao.map_or(B256::ZERO, B256::from),
            base_fee_per_gas: parent_block
                .header
                .next_block_base_fee(cfg.chain_spec.base_fee_params),
            ..Default::default()
        };

        let block = Block {
            header,
            transactions: start_tx_index
                ..start_tx_index.saturating_add(pending_transactions.len() as u64),
        };

        self.head.set(&block, state).unwrap_infallible();

        #[cfg(feature = "native")]
        {
            let mut accessory_state = state.accessory_state();
            self.pending_head
                .set(&block, &mut accessory_state)
                .unwrap_infallible();

            let mut tx_index = start_tx_index;
            for PendingTransaction {
                transaction,
                receipt,
            } in &pending_transactions
            {
                self.transactions
                    .push(transaction, &mut accessory_state)
                    .unwrap_infallible();
                self.receipts
                    .push(receipt, &mut accessory_state)
                    .unwrap_infallible();

                self.transaction_hashes
                    .set(
                        transaction.signed_transaction.hash(),
                        &tx_index,
                        &mut accessory_state,
                    )
                    .unwrap_infallible();

                tx_index += 1;
            }
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
            .unwrap_or_else(|| {
                panic!("The impossible happened: the pending block should always be set.")
            });

        let user_space_root_hash: [u8; 32] = root_hash.namespace_root(ProvableNamespace::User);
        block.header.state_root = user_space_root_hash.into();

        let sealed_block = block.seal();

        self.blocks.push(&sealed_block, state).unwrap_infallible();
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
        let transaction_pruning_threshold = config_value!("EVM_TRANSACTION_PRUNING_THRESHOLD");

        // Prune blocks
        while self.blocks.len(state)? > block_pruning_threshold {
            let block = self
                .blocks
                .remove(0, state)?
                // Safe because we already checked blocks.len()
                .expect("Impossible happened: no block available to prune");

            let block_hash = block.header.hash();
            self.block_hashes
                .remove(&block_hash, state)
                // Safe, since we keep one block_hash per block.
                .expect("Impossible happened: no block_hasha vailable to prune");
        }

        // Prune transactions
        while self.transactions.len(state)? > transaction_pruning_threshold {
            let transaction = self
                .transactions
                .remove(0, state)?
                // Safe because we already checked transactions.len()
                .expect("Impossible happened: no transaction available to prune");

            let tx_hash = transaction.signed_transaction.hash();
            self.transaction_hashes
                .remove(tx_hash, state)
                // Safe, since we keep one tx_hash per tx.
                .expect("Impossible happened: no transaction_hashes available to prune");
        }

        // Prune receipts
        while self.receipts.len(state)? > transaction_pruning_threshold {
            self.receipts
                .remove(0, state)?
                // Safe because we already checked receipts.len()
                .expect("Impossible happened: no receipts available to prune");
        }

        Ok(())
    }
}
