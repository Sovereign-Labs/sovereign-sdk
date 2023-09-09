use reth_primitives::U256;
use sov_state::WorkingSet;

use crate::evm::conversions::to_u64;
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
        let parent_block: reth_rpc_types::Block = self
            .blocks
            .get(&block_number, &mut working_set.accessory_state())
            .expect("Head block should always be set");
        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let new_pending_block = BlockEnv {
            number: block_number + 1,
            coinbase: cfg.coinbase,
            timestamp: parent_block.header.timestamp + U256::from(cfg.block_timestamp_delta),
            prevrandao: Some(da_root_hash.into()),
            basefee: reth_primitives::basefee::calculate_next_block_base_fee(
                to_u64(parent_block.header.gas_used),
                cfg.block_gas_limit,
                parent_block
                    .header
                    .base_fee_per_gas
                    .map_or(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE, to_u64),
            ),
            gas_limit: cfg.block_gas_limit,
        };
        self.pending_block.set(&new_pending_block, working_set);
    }

    pub fn end_slot_hook(&self, _root_hash: [u8; 32], working_set: &mut WorkingSet<C::Storage>) {
        // TODO implement block creation logic.

        let mut accessory_state = working_set.accessory_state();
        let mut transactions: Vec<PendingTransaction> =
            Vec::with_capacity(self.pending_transactions.len(&mut accessory_state));

        while let Some(PendingTransaction {
            mut transaction,
            mut receipt,
        }) = self.pending_transactions.pop(&mut accessory_state)
        {
            transaction.block_hash = Some(reth_primitives::H256::default());
            receipt.block_hash = transaction.block_hash;

            // TODO fill all data that is set by: from_recovered_with_block_context
            // tx.gas_price
            // tx.max_fee_per_gas
            transactions.push(PendingTransaction {
                transaction,
                receipt,
            });
        }

        transactions.reverse();

        for pending in transactions {
            self.transactions.set(
                &pending.transaction.hash,
                &pending.transaction,
                &mut accessory_state,
            );

            self.receipts.set(
                &pending.transaction.hash,
                &pending.receipt,
                &mut accessory_state,
            );
        }

        self.pending_transactions.clear(&mut accessory_state);
    }
}
