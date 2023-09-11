use sov_state::WorkingSet;

use crate::evm::transaction::BlockEnv;
use crate::experimental::PendingTransaction;
use crate::Evm;

impl<C: sov_modules_api::Context> Evm<C> {
    pub fn begin_slot_hook(
        &self,
        da_root_hash: [u8; 32],
        working_set: &mut WorkingSet<C::Storage>,
    ) {
        let block_number: u64 = self.head_number.get(working_set).unwrap();
        let parent_block = self
            .blocks
            .get(block_number as usize, &mut working_set.accessory_state())
            .expect("Head block should always be set")
            .header;
        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let new_pending_block = BlockEnv {
            number: block_number + 1,
            coinbase: cfg.coinbase,
            timestamp: parent_block.timestamp + cfg.block_timestamp_delta,
            prevrandao: Some(da_root_hash.into()),
            basefee: reth_primitives::basefee::calculate_next_block_base_fee(
                parent_block.gas_used,
                cfg.block_gas_limit,
                parent_block
                    .base_fee_per_gas
                    .unwrap_or(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE),
            ),
            gas_limit: cfg.block_gas_limit,
        };
        self.pending_block.set(&new_pending_block, working_set);
    }

    pub fn end_slot_hook(&self, _root_hash: [u8; 32], working_set: &mut WorkingSet<C::Storage>) {
        // TODO implement block creation logic.

        // let _pending_block = self
        //     .pending_block
        //     .get(working_set)
        //     .expect("Pending block should always be set");

        let mut accessory_state = working_set.accessory_state();
        let mut transactions: Vec<PendingTransaction> =
            Vec::with_capacity(self.pending_transactions.len(&mut accessory_state));

        while let Some(PendingTransaction {
            transaction,
            receipt,
        }) = self.pending_transactions.pop(&mut accessory_state)
        {
            // TODO fill all data that is set by: from_recovered_with_block_context
            // tx.gas_price
            // tx.max_fee_per_gas
            transactions.push(PendingTransaction {
                transaction,
                receipt,
            });
        }

        transactions.reverse();

        let start_tx_index = self.transactions.len(&mut accessory_state);
        let mut tx_index = start_tx_index;

        for PendingTransaction {
            transaction,
            receipt,
        } in transactions
        {
            self.transactions.push(&transaction, &mut accessory_state);
            self.receipts.push(&receipt, &mut accessory_state);

            self.transaction_hashes.set(
                &transaction.signed_transaction.hash,
                &(tx_index as u64),
                &mut accessory_state,
            );

            tx_index += 1;
        }

        self.pending_transactions.clear(&mut accessory_state);
    }
}
