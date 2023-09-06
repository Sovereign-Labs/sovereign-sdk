use reth_rpc_types::Transaction;
use sov_state::WorkingSet;

use crate::Evm;

impl<C: sov_modules_api::Context> Evm<C> {
    pub fn begin_slot_hook(
        &self,
        _da_root_hash: [u8; 32],
        _working_set: &mut WorkingSet<C::Storage>,
    ) {
    }

    pub fn end_slot_hook(&self, _root_hash: [u8; 32], working_set: &mut WorkingSet<C::Storage>) {
        // TODO implement block creation logic.

        let mut transactions: Vec<Transaction> = Vec::default();

        while let Some(mut tx) = self
            .pending_transactions
            .pop(&mut working_set.accessory_state())
        {
            tx.block_hash = Some(reth_primitives::H256::default());
            tx.block_number = Some(reth_primitives::U256::from(1));
            tx.transaction_index = Some(reth_primitives::U256::from(1));

            // TODO fill all data that is set by: from_recovered_with_block_context
            // tx.gas_price
            // tx.max_fee_per_gas
            transactions.push(tx);
        }

        for tx in transactions {
            self.transactions
                .set(&tx.hash, &tx, &mut working_set.accessory_state());
        }
    }
}
