use reth_primitives::U256;
use reth_rpc_types::Transaction;
use sov_state::WorkingSet;

use crate::evm::conversions::to_u64;
use crate::evm::transaction::BlockEnv;
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
