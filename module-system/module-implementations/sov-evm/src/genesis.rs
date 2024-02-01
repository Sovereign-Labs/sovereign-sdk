use std::collections::HashMap;

use anyhow::Result;
use reth_primitives::constants::{EMPTY_RECEIPTS, EMPTY_TRANSACTIONS};
use reth_primitives::{Address, Bloom, Bytes, EMPTY_OMMER_ROOT, H256, KECCAK_EMPTY, U256};
use revm::primitives::SpecId;
use sov_modules_api::prelude::*;
use sov_modules_api::WorkingSet;

use crate::evm::db_init::InitEvmDb;
use crate::evm::primitive_types::Block;
use crate::evm::{AccountInfo, EvmChainConfig};
use crate::Evm;

/// Evm account.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct AccountData {
    /// Account address.
    pub address: Address,
    /// Account balance.
    pub balance: U256,
    /// Code hash.
    pub code_hash: H256,
    /// Smart contract code.
    pub code: Bytes,
    /// Account nonce.
    pub nonce: u64,
}

impl AccountData {
    /// Empty code hash.
    pub fn empty_code() -> H256 {
        KECCAK_EMPTY
    }

    /// Account balance.
    pub fn balance(balance: u64) -> U256 {
        U256::from(balance)
    }
}

/// Genesis configuration.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct EvmConfig {
    /// Genesis accounts.
    pub data: Vec<AccountData>,
    /// Chain id.
    pub chain_id: u64,
    /// Limits size of contract code size.
    pub limit_contract_code_size: Option<usize>,
    /// List of EVM hardforks by block number
    pub spec: HashMap<u64, SpecId>,
    /// Coinbase where all the fees go
    pub coinbase: Address,
    /// Starting base fee.
    pub starting_base_fee: u64,
    /// Gas limit for single block
    pub block_gas_limit: u64,
    /// Genesis timestamp.
    pub genesis_timestamp: u64,
    /// Delta to add to parent block timestamp,
    pub block_timestamp_delta: u64,
    /// Base fee params.
    pub base_fee_params: reth_primitives::BaseFeeParams,
}

impl Default for EvmConfig {
    fn default() -> Self {
        Self {
            data: vec![],
            chain_id: 1,
            limit_contract_code_size: None,
            spec: vec![(0, SpecId::SHANGHAI)].into_iter().collect(),
            coinbase: Address::zero(),
            starting_base_fee: reth_primitives::constants::MIN_PROTOCOL_BASE_FEE,
            block_gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT,
            block_timestamp_delta: reth_primitives::constants::SLOT_DURATION.as_secs(),
            genesis_timestamp: 0,
            base_fee_params: reth_primitives::BaseFeeParams::ethereum(),
        }
    }
}

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
            // This will be set in finalize_hook or in the next begin_slot_hook
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use reth_primitives::Bytes;
    use revm::primitives::{Address, SpecId};

    use crate::{AccountData, EvmConfig};

    #[test]
    fn test_config_serialization() {
        let address = Address::from_str("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266").unwrap();
        let config = EvmConfig {
            data: vec![AccountData {
                address,
                balance: AccountData::balance(u64::MAX),
                code_hash: AccountData::empty_code(),
                code: Bytes::default(),
                nonce: 0,
            }],
            chain_id: 1,
            limit_contract_code_size: None,
            spec: vec![(0, SpecId::SHANGHAI)].into_iter().collect(),
            block_timestamp_delta: 1u64,
            ..Default::default()
        };

        let data = r#"
        {
            "data":[
                {
                    "address":"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                    "balance":"0xffffffffffffffff",
                    "code_hash":"0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
                    "code":"0x",
                    "nonce":0
                }],
                "chain_id":1,
                "limit_contract_code_size":null,
                "spec":{
                    "0":"SHANGHAI"
                },
                "coinbase":"0x0000000000000000000000000000000000000000",
                "starting_base_fee":7,
                "block_gas_limit":30000000,
                "genesis_timestamp":0,
                "block_timestamp_delta":1,
                "base_fee_params":{
                    "max_change_denominator":8,
                    "elasticity_multiplier":2
                }
        }"#;

        let parsed_config: EvmConfig = serde_json::from_str(data).unwrap();
        assert_eq!(config, parsed_config)
    }
}
