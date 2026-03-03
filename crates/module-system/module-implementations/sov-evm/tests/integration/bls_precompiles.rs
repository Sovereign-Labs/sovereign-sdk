//! Integration tests for EIP-2537 BLS12-381 precompiles.
//!
//! These tests verify that BLS12-381 curve operations work correctly
//! when the Prague hardfork is activated.

use std::str::FromStr;

use crate::helpers::EvmAccount;
use crate::runtime::{GenesisConfig, TestRuntime, RT, S};
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{address, hex, Address, Bytes, TxKind};
use sov_address::{EthereumAddress, FromVmAddress, MultiAddress};
use sov_evm::{
    AccountData, EthereumAuthenticator, EvmChainSpec, EvmGenesisConfig, RlpEvmTransaction, SpecId,
};
use sov_evm_test_utils::{PrecompileTester, SolCall};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{TransactionTestCase, TransactionType, TEST_DEFAULT_USER_BALANCE};

// BLS12-381 G1ADD precompile address (EIP-2537)
const BLS12_G1ADD: Address = address!("000000000000000000000000000000000000000b");

// G1 generator point in EIP-2537 format (128 bytes):
// - 64 bytes for x coordinate (left-padded 48-byte value)
// - 64 bytes for y coordinate (left-padded 48-byte value)
//
// The BLS12-381 G1 generator point coordinates:
// x = 0x17f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb
// y = 0x08b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1
const G1_GENERATOR_HEX: &str = "0000000000000000000000000000000017f1d3a73197d7942695638c4fa9ac0fc3688c4f9774b905a14e3a3f171bac586c55e83ff97a1aeffb3af00adb22c6bb0000000000000000000000000000000008b3f481e3aaa0f1a09e30ed741d8ae4fcf5e095d5d00af600db18cb2c04b3edd03cc744a2888ae40caa232946c5e7e1";

// Point at infinity (zero point) - 128 zero bytes
const G1_ZERO: [u8; 128] = [0u8; 128];

/// Helper struct for transaction creation results.
struct TxWithHash {
    tx: TransactionType<RT, S>,
}

/// Creates an EVM transaction that deploys a contract.
fn create_deploy_tx(nonce: u64, account: &EvmAccount, bytecode: Bytes) -> TxWithHash {
    let tx = TxEip1559 {
        to: TxKind::Create,
        input: bytecode,
        nonce,
        gas_limit: 3_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    };

    let signer = sov_eth_dev_signer::Signer::new(account.secret_key());
    let signed_tx = signer
        .sign_transaction(TypedTransaction::Eip1559(tx))
        .unwrap();
    let rlp = signed_tx.encoded_2718();
    let raw_tx = RawTx {
        data: borsh::to_vec(&RlpEvmTransaction { rlp }).unwrap(),
    };

    TxWithHash {
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
    }
}

/// Creates an EVM transaction that calls a contract.
fn create_contract_call_tx(
    nonce: u64,
    account: &EvmAccount,
    contract_address: Address,
    input: Bytes,
) -> TxWithHash {
    let tx = TxEip1559 {
        to: TxKind::Call(contract_address),
        input,
        nonce,
        gas_limit: 1_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    };

    let signer = sov_eth_dev_signer::Signer::new(account.secret_key());
    let signed_tx = signer
        .sign_transaction(TypedTransaction::Eip1559(tx))
        .unwrap();
    let rlp = signed_tx.encoded_2718();
    let raw_tx = RawTx {
        data: borsh::to_vec(&RlpEvmTransaction { rlp }).unwrap(),
    };

    TxWithHash {
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
    }
}

/// Sets up a test environment with Prague hardfork (BLS precompiles available).
fn setup_prague() -> (TestRunner<RT, S>, EvmAccount) {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate();
    let evm_account = EvmAccount::generate();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData::empty_with_address(evm_account.address())],
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::PRAGUE)], // Prague enables BLS precompiles
        },
        contract_creation_policy: Default::default(),
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: MultiAddress::from_vm_address(
            EthereumAddress::from_str("0x0123456789012345678901234567890123456789").unwrap(),
        ),
    };

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (runner, evm_account)
}

/// Sets up a test environment with Cancun hardfork (BLS precompiles NOT available).
fn setup_cancun() -> (TestRunner<RT, S>, EvmAccount) {
    let genesis_config = HighLevelOptimisticGenesisConfig::generate();
    let evm_account = EvmAccount::generate();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData::empty_with_address(evm_account.address())],
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::CANCUN)], // Cancun does NOT have BLS precompiles
        },
        contract_creation_policy: Default::default(),
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: MultiAddress::from_vm_address(
            EthereumAddress::from_str("0x0123456789012345678901234567890123456789").unwrap(),
        ),
    };

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (runner, evm_account)
}

/// Test that BLS12 G1ADD precompile works with Prague hardfork.
///
/// This test verifies:
/// 1. The G1ADD precompile (0x0b) is available in Prague
/// 2. Adding a point to zero returns the original point (P + 0 = P)
#[test]
fn test_bls12_g1add_works_with_prague() {
    let (mut runner, evm_account) = setup_prague();

    // Deploy the PrecompileTester contract
    let tester_address = evm_account.address().create(0);
    let deploy_tx = create_deploy_tx(
        0,
        &evm_account,
        Bytes::from(PrecompileTester::BYTECODE.to_vec()),
    );

    runner.execute_transaction(TransactionTestCase {
        input: deploy_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Contract deployment should succeed"
            );
        }),
    });

    // Test the identity property: P + 0 = P
    // Input: G1 generator point (128 bytes) + zero point (128 bytes) = 256 bytes
    let g1_generator = hex::decode(G1_GENERATOR_HEX).expect("valid hex");
    let mut precompile_input = Vec::with_capacity(256);
    precompile_input.extend_from_slice(&g1_generator);
    precompile_input.extend_from_slice(&G1_ZERO);

    // Expected output: G1 generator point (same as input point, since P + 0 = P)
    let expected_output = Bytes::from(g1_generator.clone());

    // Call assertPrecompileResult to verify the precompile returns the expected output
    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: BLS12_G1ADD,
        input: Bytes::from(precompile_input),
        expectedOutput: expected_output,
    };
    let call_data = Bytes::from(call.abi_encode());
    let call_tx = create_contract_call_tx(1, &evm_account, tester_address, call_data);

    runner.execute_transaction(TransactionTestCase {
        input: call_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "G1ADD precompile should return correct result (P + 0 = P)"
            );
        }),
    });
}

/// Test that BLS12 G1ADD precompile is NOT available before Prague (in Cancun).
///
/// In Cancun, address 0x0b is not a precompile, so calling it behaves like
/// calling an empty account - the call succeeds but returns empty data.
#[test]
fn test_bls12_g1add_not_available_in_cancun() {
    let (mut runner, evm_account) = setup_cancun();

    // Deploy the PrecompileTester contract
    let tester_address = evm_account.address().create(0);
    let deploy_tx = create_deploy_tx(
        0,
        &evm_account,
        Bytes::from(PrecompileTester::BYTECODE.to_vec()),
    );

    runner.execute_transaction(TransactionTestCase {
        input: deploy_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Contract deployment should succeed"
            );
        }),
    });

    // Same input as the Prague test
    let g1_generator = hex::decode(G1_GENERATOR_HEX).expect("valid hex");
    let mut precompile_input = Vec::with_capacity(256);
    precompile_input.extend_from_slice(&g1_generator);
    precompile_input.extend_from_slice(&G1_ZERO);

    // In Cancun, the BLS precompile doesn't exist, so calling 0x0b is like
    // calling an empty account - it succeeds but returns empty data.
    // We use assertPrecompileResult with empty expected output.
    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: BLS12_G1ADD,
        input: Bytes::from(precompile_input),
        expectedOutput: Bytes::new(), // Empty - no precompile, EOA behavior
    };
    let call_data = Bytes::from(call.abi_encode());
    let call_tx = create_contract_call_tx(1, &evm_account, tester_address, call_data);

    runner.execute_transaction(TransactionTestCase {
        input: call_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "In Cancun, calling BLS precompile address returns empty (EOA behavior)"
            );
        }),
    });
}
