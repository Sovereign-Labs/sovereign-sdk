use std::collections::HashMap;

use anyhow::Result;
use reth_primitives::constants::{EMPTY_RECEIPTS, EMPTY_ROOT_HASH, EMPTY_TRANSACTIONS};
use reth_primitives::revm_primitives::{AccountInfo, Address, SpecId, B256, U256};
use reth_primitives::{Bloom, Bytes, EMPTY_OMMER_ROOT_HASH, KECCAK_EMPTY};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_bank::config_gas_token_id;
use sov_modules_api::macros::config_value;
use sov_modules_api::{Amount, GenesisState, Module, Spec};

use crate::evm::db_init::InitEvmDb;
use crate::evm::primitive_types::Block;
use crate::evm::EvmChainConfig;
use crate::{to_rollup_address, Evm};

/// Evm account.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct AccountData {
    /// Account address.
    pub address: Address,
    /// Account balance.
    pub balance: U256,
    /// Code hash.
    pub code_hash: B256,
    /// Smart contract code.
    pub code: Bytes,
    /// Account nonce.
    pub nonce: u64,
}

impl AccountData {
    /// Empty code hash.
    pub fn empty_code() -> B256 {
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
    /// List of EVM hard forks by block number
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
    pub base_fee_params: alloy_eips::eip1559::BaseFeeParams,
}

impl Default for EvmConfig {
    fn default() -> Self {
        Self {
            data: vec![],
            chain_id: config_value!("CHAIN_ID"),
            limit_contract_code_size: None,
            spec: vec![(0, SpecId::SHANGHAI)].into_iter().collect(),
            coinbase: Address::ZERO,
            starting_base_fee: reth_primitives::constants::MIN_PROTOCOL_BASE_FEE,
            block_gas_limit: reth_primitives::constants::ETHEREUM_BLOCK_GAS_LIMIT,
            block_timestamp_delta: reth_primitives::constants::SLOT_DURATION.as_secs(),
            genesis_timestamp: 0,
            base_fee_params: alloy_eips::eip1559::BaseFeeParams::ethereum(),
        }
    }
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    pub(crate) fn init_module(
        &mut self,
        config: &<Self as Module>::Config,
        state: &mut impl GenesisState<S>,
    ) -> Result<()> {
        for mut acc in config.data.clone() {
            let rollup_address: <S as Spec>::Address = to_rollup_address::<S>(acc.address);
            let bank_balance =
                self.bank_module
                    .get_balance_of(&rollup_address, config_gas_token_id(), state)?;

            assert!(
                !(acc.balance != U256::ZERO && bank_balance.is_some()),
                "EVM account balance can only be set from one genesis config to avoid conflicts. 
                Choose either the Bank or the EVM module genesis config."
            );

            if acc.balance != U256::ZERO {
                self.bank_module.override_gas_balance(
                    Amount::new(acc.balance.try_into().unwrap()),
                    &rollup_address,
                    state,
                )?;
                acc.balance = U256::ZERO;
            }

            let mut evm_db = self.get_db(state);
            evm_db.insert_account_info(
                acc.address,
                AccountInfo {
                    balance: acc.balance,
                    code_hash: acc.code_hash,
                    nonce: acc.nonce,
                    code: None,
                },
            );

            if !acc.code.is_empty() {
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

        self.cfg.set(&chain_cfg, state)?;

        let header = reth_primitives::Header {
            parent_hash: B256::default(),
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary: config.coinbase,
            // This will be set in finalize_hook or in the next begin_rollup_block_hook
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
            mix_hash: B256::default(),
            nonce: 0,
            base_fee_per_gas: Some(config.starting_base_fee),
            extra_data: Bytes::default(),
            // EIP-4844 related fields
            blob_gas_used: None,
            excess_blob_gas: None,
            // EIP-4788 related field
            // unrelated for rollups
            parent_beacon_block_root: None,
            // If Prague is activated at genesis we set requests root to an empty trie root.
            requests_root: Some(EMPTY_ROOT_HASH),
        };

        let block = Block {
            header,
            transactions: 0u64..0u64,
        };

        self.head.set(&block, state)?;
        #[cfg(feature = "native")]
        {
            self.pending_head.set(&block, state)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use reth_primitives::revm_primitives::{Address, SpecId};
    use reth_primitives::Bytes;
    use sov_modules_api::prelude::serde_json;

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
            chain_id: 4321, // Use a hard-coded value instead of config_value!("CHAIN_ID") since the string below is hard-coded
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
                "chain_id":4321,
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
        assert_eq!(config, parsed_config);
    }
}
