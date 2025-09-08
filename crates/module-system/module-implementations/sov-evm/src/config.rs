use alloy_eips::eip1559::{ETHEREUM_BLOCK_GAS_LIMIT_30M, MIN_PROTOCOL_BASE_FEE};
use alloy_eips::merge::SLOT_DURATION;
use alloy_primitives::Address;
use revm::primitives::hardfork::SpecId;
use sov_modules_api::macros::config_value;

use crate::AccountData;

/// Core EVM chain parameters shared between genesis and runtime
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct EvmChainSpec {
    /// Unique chain identifier
    pub chain_id: u64,
    /// Maximum contract code size (None = unlimited)
    pub limit_contract_code_size: Option<usize>,
    /// Address where transaction fees are collected
    pub coinbase: Address,
    /// Maximum gas allowed per block
    pub block_gas_limit: u64,
    /// Seconds to add to parent block timestamp
    pub block_timestamp_delta: u64,
    /// Hard fork activation schedule (block number -> fork ID)
    pub hardforks: Vec<(u64, SpecId)>,
}

/// Genesis configuration for EVM module initialization
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Eq, PartialEq)]
pub struct EvmGenesisConfig {
    /// Initial account states
    pub accounts: Vec<AccountData>,
    /// Initial base fee for first block
    pub initial_base_fee: u64,
    /// Timestamp of genesis block
    pub genesis_timestamp: u64,
    /// Core chain parameters
    pub chain_spec: EvmChainSpec,
}

impl Default for EvmChainSpec {
    fn default() -> Self {
        Self {
            chain_id: config_value!("CHAIN_ID"),
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: ETHEREUM_BLOCK_GAS_LIMIT_30M,
            block_timestamp_delta: SLOT_DURATION.as_secs(),
            hardforks: vec![(0, SpecId::SHANGHAI)],
        }
    }
}

impl Default for EvmGenesisConfig {
    fn default() -> Self {
        Self {
            accounts: vec![],
            initial_base_fee: MIN_PROTOCOL_BASE_FEE,
            genesis_timestamp: 0,
            chain_spec: EvmChainSpec::default(),
        }
    }
}

/// Runtime configuration for EVM execution
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct EvmRuntimeConfig {
    /// Core chain parameters
    pub chain_spec: crate::EvmChainSpec,
    /// Sorted hard fork schedule for efficient runtime lookup
    /// (block number, fork ID) ordered by block number
    pub hardforks: Vec<(u64, SpecId)>,
}

impl Default for EvmRuntimeConfig {
    fn default() -> EvmRuntimeConfig {
        let chain_spec = crate::EvmChainSpec::default();
        // Clone hardforks from chain_spec for runtime use
        let hardforks = chain_spec.hardforks.clone();

        EvmRuntimeConfig {
            chain_spec,
            hardforks,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::{Address, Bytes};
    use revm::primitives::hardfork::SpecId;
    use sov_modules_api::prelude::serde_json;

    use crate::{AccountData, EvmGenesisConfig};

    #[test]
    fn test_config_serialization() {
        let address = Address::from_str("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266").unwrap();
        let config = EvmGenesisConfig {
            accounts: vec![AccountData {
                address,
                code_hash: AccountData::empty_code(),
                code: Bytes::default(),
            }],
            chain_spec: crate::EvmChainSpec {
                chain_id: 4321, // Use a hard-coded value instead of config_value!("CHAIN_ID") since the string below is hard-coded
                limit_contract_code_size: None,
                block_timestamp_delta: 1u64,
                hardforks: vec![(0, SpecId::SHANGHAI)],
                ..Default::default()
            },
            ..Default::default()
        };

        let data = r#"
        {
            "accounts":[
                {
                    "address":"0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266",
                    "code_hash":"0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470",
                    "code":"0x",
                    "nonce":0
                }],
                "initial_base_fee":7,
                "genesis_timestamp":0,
                "chain_spec":{
                    "chain_id":4321,
                    "limit_contract_code_size":null,
                    "coinbase":"0x0000000000000000000000000000000000000000",
                    "block_gas_limit":30000000,
                    "block_timestamp_delta":1,
                    "hardforks":[[0,"SHANGHAI"]]
                }
        }"#;

        let parsed_config: EvmGenesisConfig = serde_json::from_str(data).unwrap();
        assert_eq!(config, parsed_config);
    }
}
