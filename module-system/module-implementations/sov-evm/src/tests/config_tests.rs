use std::str::FromStr;

use crate::{AccountData, EvmConfig};
use reth_primitives::{Address, Bytes};
use revm::primitives::SpecId;

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
