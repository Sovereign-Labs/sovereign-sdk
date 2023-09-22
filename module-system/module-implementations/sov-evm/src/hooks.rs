use reth_primitives::{Bloom, Bytes, U256};
use sov_modules_api::{AccessoryWorkingSet, Spec, WorkingSet};
use sov_state::Storage;

use crate::evm::primitive_types::{Block, BlockEnv};
use crate::experimental::PendingTransaction;
use crate::Evm;

impl<C: sov_modules_api::Context> Evm<C>
where
    <C::Storage as Storage>::Root: Into<[u8; 32]>,
{
    pub fn begin_slot_hook(&self, da_root_hash: [u8; 32], working_set: &mut WorkingSet<C>) {
        let parent_block = self
            .head
            .get(working_set)
            .expect("Head block should always be set");

        // parent_block.header.state_root = root_hash.into();
        // self.head.set(&parent_block, working_set);

        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let new_pending_block = BlockEnv {
            number: parent_block.header.number + 1,
            coinbase: cfg.coinbase,
            timestamp: parent_block.header.timestamp + cfg.block_timestamp_delta,
            prevrandao: da_root_hash.into(),
            basefee: parent_block
                .header
                .next_block_base_fee(cfg.base_fee_params)
                .unwrap(),
            gas_limit: cfg.block_gas_limit,
        };
        self.pending_block.set(&new_pending_block, working_set);
    }

    pub fn end_slot_hook(&self, working_set: &mut WorkingSet<C>) {
        let cfg = self.cfg.get(working_set).unwrap_or_default();

        let pending_block = self
            .pending_block
            .get(working_set)
            .expect("Pending block should always be set");

        let parent_block = self
            .head
            .get(working_set)
            .expect("Head block should always be set")
            .seal();

        let expected_block_number = parent_block.header.number + 1;
        assert_eq!(
            pending_block.number, expected_block_number,
            "Pending head must be set to block {}, but found block {}",
            expected_block_number, pending_block.number
        );

        let pending_transactions: Vec<PendingTransaction> =
            self.pending_transactions.iter(working_set).collect();

        self.pending_transactions.clear(working_set);

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
            parent_hash: parent_block.header.hash,
            timestamp: pending_block.timestamp,
            number: pending_block.number,
            ommers_hash: reth_primitives::constants::EMPTY_OMMER_ROOT,
            beneficiary: parent_block.header.beneficiary,
            // This will be set in finalize_slot_hook or in the next begin_slot_hook
            state_root: reth_primitives::constants::KECCAK_EMPTY,
            transactions_root: reth_primitives::proofs::calculate_transaction_root(
                transactions.as_slice(),
            ),
            receipts_root: reth_primitives::proofs::calculate_receipt_root(receipts.as_slice()),
            withdrawals_root: None,
            logs_bloom: receipts
                .iter()
                .fold(Bloom::zero(), |bloom, r| bloom | r.bloom),
            difficulty: U256::ZERO,
            gas_limit: pending_block.gas_limit,
            gas_used,
            mix_hash: pending_block.prevrandao,
            nonce: 0,
            base_fee_per_gas: parent_block.header.next_block_base_fee(cfg.base_fee_params),
            extra_data: Bytes::default(),
            // EIP-4844 related fields
            // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
            blob_gas_used: None,
            excess_blob_gas: None,
            // EIP-4788 related field
            // unrelated for rollups
            parent_beacon_block_root: None,
        };

        let block = Block {
            header,
            transactions: start_tx_index..start_tx_index + pending_transactions.len() as u64,
        };

        self.head.set(&block, working_set);

        let mut accessory_state = working_set.accessory_state();
        self.pending_head.set(&block, &mut accessory_state);

        let mut tx_index = start_tx_index;
        for PendingTransaction {
            transaction,
            receipt,
        } in &pending_transactions
        {
            self.transactions.push(transaction, &mut accessory_state);
            self.receipts.push(receipt, &mut accessory_state);

            self.transaction_hashes.set(
                &transaction.signed_transaction.hash,
                &tx_index,
                &mut accessory_state,
            );

            tx_index += 1
        }

        self.pending_transactions.clear(working_set);
    }

    pub fn finalize_slot_hook(
        &self,
        root_hash: &<<C as Spec>::Storage as Storage>::Root,
        accesorry_working_set: &mut AccessoryWorkingSet<C>,
    ) {
        let expected_block_number = self.blocks.len(accesorry_working_set) as u64;

        let mut block = self
            .pending_head
            .get(accesorry_working_set)
            .unwrap_or_else(|| {
                panic!(
                    "Pending head must be set to block {}, but was empty",
                    expected_block_number
                )
            });

        assert_eq!(
            block.header.number, expected_block_number,
            "Pending head must be set to block {}, but found block {}",
            expected_block_number, block.header.number
        );

        let root_hash_bytes: [u8; 32] = root_hash.clone().into();
        block.header.state_root = root_hash_bytes.into();

        let sealed_block = block.seal();

        self.blocks.push(&sealed_block, accesorry_working_set);
        self.block_hashes.set(
            &sealed_block.header.hash,
            &sealed_block.header.number,
            accesorry_working_set,
        );
        self.pending_head.delete(accesorry_working_set);
    }
}
