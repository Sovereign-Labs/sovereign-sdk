use reth_primitives::U256;
use reth_rpc_types::Transaction;
use sov_state::WorkingSet;

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
            .unwrap();
        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let new_pending_block = BlockEnv {
            number: block_number + 1,
            coinbase: cfg.coinbase,

            // TODO: simplify this conversion by doing something with Bytes32
            // TODO simplify conversion fro U256 to u64
            // Reth rpc types keep stuff as U256, even when actually only u64 makes sense:
            // block_number, timespamp, gas_used, base_fee_per_gas
            timestamp: (parent_block.header.timestamp + U256::from(cfg.block_timestamp_delta))
                .to_le_bytes(),
            prevrandao: Some(da_root_hash),
            basefee: {
                let base_fee = reth_primitives::basefee::calculate_next_block_base_fee(
                    parent_block.header.gas_used.as_limbs()[0],
                    cfg.block_gas_limit,
                    parent_block
                        .header
                        .base_fee_per_gas
                        .unwrap_or(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE_U256)
                        .as_limbs()[0],
                );

                U256::from_limbs([base_fee, 0u64, 0u64, 0u64]).to_le_bytes()
            },
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
