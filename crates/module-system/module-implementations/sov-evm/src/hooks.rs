use reth_primitives::revm_primitives::{B256, U256};
use reth_primitives::{Bloom, Bytes};
use sov_modules_api::prelude::UnwrapInfallible;
#[cfg(feature = "native")]
use sov_modules_api::{AccessoryStateReaderAndWriter, FinalizeHook};
use sov_modules_api::{BlockHooks, Spec, StateCheckpoint};
use sov_state::{ProvableNamespace, StateRoot, Storage};

use crate::evm::primitive_types::Block;
use crate::{BlockEnv, Evm, PendingTransaction};

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

        let cfg = self.cfg.get(state).unwrap_infallible().unwrap_or_default();

        let new_pending_env = BlockEnv {
            number: U256::from(parent_block.header.number.wrapping_add(1)),
            coinbase: cfg.coinbase,
            timestamp: U256::from(
                parent_block
                    .header
                    .timestamp
                    .saturating_add(cfg.block_timestamp_delta),
            ),
            // WARNING: `prevrandao`` value is predictable up to [`DEFERRED_SLOTS_COUNT`] in advance,
            // Users should follow the same best practice that they would on Ethereum and use future randomness.
            // See: https://eips.ethereum.org/EIPS/eip-4399#tips-for-application-developers
            prevrandao: Some(B256::from(pre_state_user_root)),
            basefee: U256::from(
                parent_block
                    .header
                    .next_block_base_fee(cfg.base_fee_params)
                    .unwrap(),
            ),
            gas_limit: U256::from(cfg.block_gas_limit),
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
        let cfg = self.cfg.get(state).unwrap_infallible().unwrap_or_default();

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
            block_env.number.to::<u64>(),
            expected_block_number,
            "Pending head must be set to block {}, but found block {}",
            expected_block_number,
            block_env.number
        );

        let pending_transactions: Vec<PendingTransaction> =
            self.pending_transactions.collect_infallible(state);

        self.pending_transactions.clear(state).unwrap_infallible();

        let start_tx_index = parent_block.transactions.end;

        let gas_used = pending_transactions
            .last()
            .map_or(0u64, |tx| tx.receipt.receipt.cumulative_gas_used);

        let transactions: Vec<&reth_primitives::TransactionSigned> = pending_transactions
            .iter()
            .map(|tx| &tx.transaction.signed_transaction)
            .collect();

        let receipts: Vec<reth_primitives::ReceiptWithBloom> = pending_transactions
            .iter()
            .map(|tx| tx.receipt.receipt.clone().with_bloom())
            .collect();

        let header = reth_primitives::Header {
            parent_hash: parent_block.header.hash(),
            timestamp: block_env.timestamp.to(),
            number: block_env.number.to(),
            ommers_hash: reth_primitives::constants::EMPTY_OMMER_ROOT_HASH,
            beneficiary: parent_block.header.beneficiary,
            // This will be set in finalize_hook or in the next begin_rollup_block_hook
            state_root: reth_primitives::constants::KECCAK_EMPTY,
            transactions_root: reth_primitives::proofs::calculate_transaction_root(
                transactions.as_slice(),
            ),
            receipts_root: reth_primitives::proofs::calculate_receipt_root(receipts.as_slice()),
            withdrawals_root: None,
            logs_bloom: receipts
                .iter()
                .fold(Bloom::ZERO, |bloom, r| bloom | r.bloom),
            difficulty: U256::ZERO,
            gas_limit: block_env.gas_limit.to(),
            gas_used,
            mix_hash: block_env.prevrandao.map_or(B256::ZERO, B256::from),
            nonce: 0,
            base_fee_per_gas: parent_block.header.next_block_base_fee(cfg.base_fee_params),
            extra_data: Bytes::default(),
            // EIP-4844 related fields
            blob_gas_used: None,
            excess_blob_gas: None,
            // EIP-4788 related field
            // unrelated for rollups
            parent_beacon_block_root: None,
            // EIP-7685: TODO: Sovereign does not yet support it: https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1131
            requests_root: None,
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
                        &transaction.signed_transaction.hash,
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
        let expected_block_number = self.blocks.len(state).unwrap_infallible();

        let mut block = self
            .pending_head
            .get(state)
            .unwrap_infallible()
            .unwrap_or_else(|| {
                panic!("Pending head must be set to block {expected_block_number}, but was empty")
            });

        assert_eq!(
            block.header.number, expected_block_number,
            "Pending head must be set to block {}, but found block {}",
            expected_block_number, block.header.number
        );
        let user_space_root_hash: [u8; 32] = root_hash.namespace_root(ProvableNamespace::User);
        block.header.state_root = user_space_root_hash.into();

        let sealed_block = block.seal();

        self.blocks.push(&sealed_block, state).unwrap_infallible();
        self.block_hashes
            .set(
                &sealed_block.header.hash(),
                &sealed_block.header.number,
                state,
            )
            .unwrap_infallible();
        self.pending_head.delete(state).unwrap_infallible();
    }
}
