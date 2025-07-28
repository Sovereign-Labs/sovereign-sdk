use alloy_eips::eip1559::BaseFeeParams;
use reth_primitives::constants::{
    EMPTY_RECEIPTS, EMPTY_ROOT_HASH, EMPTY_TRANSACTIONS, ETHEREUM_BLOCK_GAS_LIMIT,
};
use reth_primitives::{
    Address, Bloom, Bytes, Header, B256, EMPTY_OMMER_ROOT_HASH, KECCAK_EMPTY, U256,
};
use revm::Database;
use sov_evm::{AccountData, Evm, EvmChainConfig, EvmConfig, SpecId};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;

use crate::helpers::setup;
use crate::runtime::{GenesisConfig, RT, S};

#[test]
fn test_genesis_data() {
    let cfg = default_config();
    let runner = basic_setup(cfg.clone());

    runner.query_visible_state(move |state| {
        let evm = Evm::<S>::default();
        let account = &cfg.data[0];
        let account_info = evm.get_db(state).basic(account.address).unwrap().unwrap();

        assert_eq!(
            &account_info,
            &reth_primitives::revm_primitives::AccountInfo {
                balance: account.balance,
                code_hash: account.code_hash,
                nonce: account.nonce,
                code: None,
            }
        );
    });
}

#[test]
fn test_genesis_cfg() {
    let cfg = default_config();
    let runner = basic_setup(cfg.clone());

    runner.query_visible_state(move |state| {
        let evm = Evm::<S>::default();

        assert_eq!(
            evm.cfg(state).unwrap().unwrap(),
            EvmChainConfig {
                spec: vec![(0, SpecId::BERLIN), (1, SpecId::SHANGHAI)],
                chain_id: 1000,
                block_gas_limit: ETHEREUM_BLOCK_GAS_LIMIT,
                block_timestamp_delta: 2,
                coinbase: Address::from([3u8; 20]),
                limit_contract_code_size: Some(5000),
                base_fee_params: BaseFeeParams::ethereum(),
            }
        );
    });
}

#[test]
fn test_empty_spec_defaults_to_shanghai() {
    let mut cfg = default_config();
    cfg.spec.clear();
    let runner = basic_setup(cfg);

    runner.query_visible_state(move |state| {
        let evm = Evm::<S>::default();
        let evm_cfg = evm.cfg(state).unwrap().unwrap();
        assert_eq!(evm_cfg.spec, vec![(0, SpecId::SHANGHAI)]);
    });
}

#[test]
#[should_panic(expected = "EVM spec must start from block 0")]
fn test_cfg_missing_specs() {
    let cfg = EvmConfig {
        spec: vec![(5, SpecId::BERLIN)].into_iter().collect(),
        ..Default::default()
    };
    let _ = basic_setup(cfg);
}

#[test]
#[should_panic(expected = "Cancun is not supported")]
fn test_cancun_is_unsupported() {
    let cfg = EvmConfig {
        spec: vec![(0, SpecId::CANCUN)].into_iter().collect(),
        ..Default::default()
    };
    let _ = basic_setup(cfg);
}

#[test]
fn test_genesis_block() {
    let (runner, _, _, _) = setup();
    let beneficiary = Address::new([0u8; 20]);

    runner.query_visible_state(move |state| {
        let evm = Evm::<S>::default();

        let actual_block = &evm.blocks(state)[0_usize];
        let expected_header = Header {
            parent_hash: B256::default(),
            state_root: actual_block.header().header().state_root,
            transactions_root: EMPTY_TRANSACTIONS,
            receipts_root: EMPTY_RECEIPTS,
            logs_bloom: Bloom::default(),
            difficulty: U256::ZERO,
            number: 0,
            gas_limit: ETHEREUM_BLOCK_GAS_LIMIT,
            gas_used: 0,
            timestamp: 0,
            extra_data: Bytes::default(),
            mix_hash: B256::default(),
            nonce: 0,
            base_fee_per_gas: Some(7),
            ommers_hash: EMPTY_OMMER_ROOT_HASH,
            beneficiary,
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_root: Some(EMPTY_ROOT_HASH),
        };

        assert_eq!(actual_block.header().header(), &expected_header);

        let txns = actual_block.transactions();
        assert_eq!(txns.start, 0);
        assert_eq!(txns.end, 0);
    });
}

fn default_config() -> EvmConfig {
    EvmConfig {
        data: vec![AccountData {
            address: Address::from([1u8; 20]),
            balance: U256::from(1000000000),
            code_hash: KECCAK_EMPTY,
            code: Bytes::default(),
            nonce: 0,
        }],
        spec: vec![(0, SpecId::BERLIN), (1, SpecId::SHANGHAI)]
            .into_iter()
            .collect(),
        chain_id: 1000,
        block_gas_limit: ETHEREUM_BLOCK_GAS_LIMIT,
        block_timestamp_delta: 2,
        genesis_timestamp: 50,
        coinbase: Address::from([3u8; 20]),
        limit_contract_code_size: Some(5000),
        starting_base_fee: 70,
        base_fee_params: BaseFeeParams::ethereum(),
    }
}

fn basic_setup(cfg: EvmConfig) -> TestRunner<RT, S> {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate();
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), cfg);

    TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default())
}
