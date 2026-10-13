//! A contract deployment rejected by the contract-creation allowlist must charge the same gas on
//! every node. The allowlist check does a metered read per touched contract until it finds the
//! newly created one, so it must visit the touched accounts in a deterministic order.
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::{keccak256, Address, Bytes, TxKind};
use sov_address::{EthereumAddress, MultiAddress};
use sov_evm::{
    AccountData, ContractCreationPolicy, EthereumAuthenticator, EvmChainSpec, EvmGenesisConfig,
    SpecId,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_rollup_interface::stf::TxEffect;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{TransactionTestCase, TransactionType, TEST_DEFAULT_USER_BALANCE};

use crate::helpers::EvmAccount;
use crate::runtime::{GenesisConfig, TestRuntime, RT, S};

const EXISTING_CONTRACTS: u8 = 8;
const RUNS: usize = 8;

fn existing_contract(i: u8) -> Address {
    Address::new([0x20 + i; 20])
}

/// Initcode that runs `EXTCODESIZE` on every existing contract (so they all end up in the EVM
/// state changes), then returns the 1-byte runtime code `0x00`.
fn initcode_touching_existing_contracts() -> Vec<u8> {
    let mut code = Vec::new();
    for i in 0..EXISTING_CONTRACTS {
        code.push(0x73); // PUSH20
        code.extend_from_slice(existing_contract(i).as_slice());
        code.push(0x3b); // EXTCODESIZE
        code.push(0x50); // POP
    }
    // PUSH1 1; PUSH1 0; RETURN
    code.extend_from_slice(&[0x60, 0x01, 0x60, 0x00, 0xf3]);
    code
}

#[test]
fn rejected_contract_creation_charges_deterministic_gas() {
    let from = EvmAccount::generate();
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let runtime_code = Bytes::from(vec![0x00u8]);
    let mut accounts = vec![AccountData::empty_with_address(from.address())];
    for i in 0..EXISTING_CONTRACTS {
        accounts.push(AccountData {
            address: existing_contract(i),
            code_hash: keccak256(&runtime_code),
            code: runtime_code.clone(),
            nonce: 1,
            storage: Default::default(),
        });
    }
    let evm_config = EvmGenesisConfig {
        accounts,
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::CANCUN)],
        },
        // The sender is not on the allowlist.
        contract_creation_policy: ContractCreationPolicy::Allowlist(BTreeSet::from([
            Address::new([0x99; 20]),
        ])),
        enabled_custom_precompiles: Default::default(),
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: admin.address(),
    };
    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);
    genesis
        .bank
        .gas_token_config
        .as_mut()
        .expect("Test genesis has a gas token config")
        .address_and_balances
        .push((
            MultiAddress::Vm(EthereumAddress::from(from.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));

    let tx = TxEip1559 {
        to: TxKind::Create,
        nonce: 0,
        gas_limit: 1_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        input: Bytes::from(initcode_touching_existing_contracts()),
        ..Default::default()
    };
    let (signed, _tx_env) = from.sign(TypedTransaction::Eip1559(tx));
    let raw_tx = borsh::to_vec(&signed).unwrap();

    let gas_used = Arc::new(Mutex::new(BTreeSet::new()));
    for _ in 0..RUNS {
        let mut runner: TestRunner<RT, S> = TestRunner::new_with_genesis(
            genesis.clone().into_genesis_params(),
            TestRuntime::default(),
        );
        let gas_used = gas_used.clone();
        runner.execute_transaction(TransactionTestCase {
            input: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(RawTx {
                data: raw_tx.clone(),
            })),
            assert: Box::new(move |result, _state| {
                let TxEffect::Reverted(contents) = &result.tx_receipt else {
                    panic!("Contract creation by a non-allowlisted sender must revert");
                };
                gas_used
                    .lock()
                    .unwrap()
                    .insert(contents.gas_used.as_ref().to_vec());
            }),
        });
    }

    let gas_used = gas_used.lock().unwrap().clone();
    assert_eq!(
        gas_used.len(),
        1,
        "Identical tx on identical state must charge identical gas: {gas_used:?}"
    );
}
