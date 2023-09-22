use anyhow::Result;
use reth_primitives::constants::{EMPTY_RECEIPTS, EMPTY_TRANSACTIONS};
use reth_primitives::{Bloom, Bytes, EMPTY_OMMER_ROOT, H256, KECCAK_EMPTY, U256};
use revm::primitives::SpecId;
use sov_modules_api::WorkingSet;

use crate::evm::db_init::InitEvmDb;
use crate::evm::primitive_types::Block;
use crate::evm::{AccountInfo, EvmChainConfig};
use crate::Evm;

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn init_module(
        &self,
        config: &<Self as sov_modules_api::Module>::Config,
        working_set: &mut WorkingSet<C>,
    ) -> Result<()> {
        let mut evm_db = self.get_db(working_set);

        for acc in &config.data {
            evm_db.insert_account_info(
                acc.address,
                AccountInfo {
                    balance: acc.balance,
                    code_hash: acc.code_hash,
                    nonce: acc.nonce,
                },
            );

            if acc.code.len() > 0 {
                evm_db.insert_code(acc.code_hash, acc.code.clone());
            }
        }

        let mut spec = config
            .spec
            .iter()
            .map(|(k, v)| {
                // https://github.com/Sovereign-Labs/sovereign-sdk/issues/912
                if *v == SpecId::CANCUN {
                    panic!("Cancun is not supported");
                }

                (*k, *v)
            })
            .collect::<Vec<_>>();

        spec.sort_by(|a, b| a.0.cmp(&b.0));

        if spec.is_empty() {
            spec.push((0, SpecId::SHANGHAI));
        } else if spec[0].0 != 0u64 {
            panic!("EVM spec must start from block 0");
        }

        let chain_cfg = EvmChainConfig {
            chain_id: config.chain_id,
            limit_contract_code_size: config.limit_contract_code_size,
            spec,
            coinbase: config.coinbase,
            block_gas_limit: config.block_gas_limit,
            block_timestamp_delta: config.block_timestamp_delta,
            base_fee_params: config.base_fee_params,
        };

        self.cfg.set(&chain_cfg, working_set);

        let header = reth_primitives::Header {
            parent_hash: H256::default(),
            ommers_hash: EMPTY_OMMER_ROOT,
            beneficiary: config.coinbase,
            // This will be set in finalize_slot_hook or in the next begin_slot_hook
            state_root: KECCAK_EMPTY,
            transactions_root: EMPTY_TRANSACTIONS,
            receipts_root: EMPTY_RECEIPTS,
            withdrawals_root: None,
            logs_bloom: Bloom::default(),
            difficulty: U256::ZERO,
            number: 0,
            gas_limit: config.block_gas_limit,
            gas_used: 0,
            timestamp: config.genesis_timestamp,
            mix_hash: H256::default(),
            nonce: 0,
            base_fee_per_gas: Some(config.starting_base_fee),
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
            transactions: 0u64..0u64,
        };

        self.head.set(&block, working_set);
        self.pending_head
            .set(&block, &mut working_set.accessory_state());

        Ok(())
    }
}
