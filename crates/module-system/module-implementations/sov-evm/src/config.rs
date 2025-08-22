use std::collections::HashMap;

use alloy_eips::eip1559::{ETHEREUM_BLOCK_GAS_LIMIT_30M, MIN_PROTOCOL_BASE_FEE};
use alloy_eips::merge::SLOT_DURATION;
use alloy_primitives::Address;
use revm::primitives::hardfork::SpecId;
use sov_modules_api::macros::config_value;

use crate::AccountData;

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
            starting_base_fee: MIN_PROTOCOL_BASE_FEE,
            block_gas_limit: ETHEREUM_BLOCK_GAS_LIMIT_30M,
            block_timestamp_delta: SLOT_DURATION.as_secs(),
            genesis_timestamp: 0,
            base_fee_params: alloy_eips::eip1559::BaseFeeParams::ethereum(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use alloy_primitives::{Address, Bytes};
    use revm::primitives::hardfork::SpecId;
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
