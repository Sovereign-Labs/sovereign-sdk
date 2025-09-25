use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_consensus::{BlockHeader, Header};
use alloy_eips::eip1559::ETHEREUM_BLOCK_GAS_LIMIT_30M;
use alloy_primitives::{Address, Bytes, U256};
use revm::state::AccountInfo;
use revm::Database;
use sov_evm::{AccountData, Evm, EvmGenesisConfig, EvmRuntimeConfig, SpecId};
use sov_modules_api::prelude::UnwrapInfallible;
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
        let account = &cfg.accounts[0];
        let account_info = evm.get_db(state).basic(account.address).unwrap().unwrap();

        assert_eq!(
            &account_info,
            &AccountInfo {
                balance: U256::ZERO,
                code_hash: account.code_hash,
                nonce: 0,
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
            evm.cfg_infallible(state),
            EvmRuntimeConfig {
                chain_spec: sov_evm::EvmChainSpec {
                    block_gas_limit: ETHEREUM_BLOCK_GAS_LIMIT_30M,
                    coinbase: Address::from([3u8; 20]),
                    limit_contract_code_size: Some(5000),
                    hardforks: vec![(0, SpecId::BERLIN), (1, SpecId::CANCUN)],
                },
                hardforks: vec![(0, SpecId::BERLIN), (1, SpecId::CANCUN)],
            }
        );
    });
}

#[test]
fn test_empty_spec_defaults_to_cancun() {
    let mut cfg = default_config();
    cfg.chain_spec.hardforks.clear();
    let runner = basic_setup(cfg);

    runner.query_visible_state(move |state| {
        let evm = Evm::<S>::default();
        let evm_cfg = evm.cfg_infallible(state);
        assert_eq!(evm_cfg.hardforks, vec![(0, SpecId::CANCUN)]);
    });
}

#[test]
#[should_panic(expected = "EVM spec must start from block 0")]
fn test_cfg_missing_specs() {
    let mut cfg = EvmGenesisConfig::default();
    cfg.chain_spec.hardforks = vec![(5, SpecId::BERLIN)];
    let _ = basic_setup(cfg);
}

#[test]
fn test_genesis_block() {
    let (runner, _, _) = setup();
    let beneficiary = Address::new([0u8; 20]);

    runner.query_visible_state(move |state| {
        let evm = Evm::<S>::default();

        let actual_block = &evm.blocks.get(&0, state).unwrap_infallible().unwrap();
        let expected_header = Header {
            state_root: actual_block.header().state_root(),
            gas_limit: ETHEREUM_BLOCK_GAS_LIMIT_30M,
            beneficiary,
            excess_blob_gas: Some(0),
            ..Default::default()
        };

        assert_eq!(actual_block.header().inner(), &expected_header);

        let txns = actual_block.transactions();
        assert_eq!(txns.start, 0);
        assert_eq!(txns.end, 0);
    });
}

fn default_config() -> EvmGenesisConfig {
    EvmGenesisConfig {
        accounts: vec![AccountData {
            address: Address::from([1u8; 20]),
            code_hash: KECCAK_EMPTY,
            code: Bytes::default(),
        }],
        initial_base_fee: 70,
        genesis_timestamp: 50,
        chain_spec: sov_evm::EvmChainSpec {
            block_gas_limit: ETHEREUM_BLOCK_GAS_LIMIT_30M,
            coinbase: Address::from([3u8; 20]),
            limit_contract_code_size: Some(5000),
            hardforks: vec![(0, SpecId::BERLIN), (1, SpecId::CANCUN)],
        },
    }
}

fn basic_setup(cfg: EvmGenesisConfig) -> TestRunner<RT, S> {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate();
    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), cfg);

    TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default())
}
