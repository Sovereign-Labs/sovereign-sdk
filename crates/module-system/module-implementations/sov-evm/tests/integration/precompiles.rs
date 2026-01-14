//! Integration tests for sovereign precompiles.
//!
//! Tests:
//! - Precompile calls fail when not enabled
//! - Precompiles work when enabled at genesis
//! - Precompiles work when enabled via admin transaction
//! - Bank balance precompile returns correct values

use std::str::FromStr;

use crate::helpers::{EvmAccount, TxWithNonceAndHash};
use crate::runtime::{GenesisConfig, TestRuntime, RT, S};
use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{Address, Bytes, TxKind, U256};
use sov_evm_test_utils::SolCall;
use sov_address::{EthereumAddress, FromVmAddress, MultiAddress};
use sov_bank::{config_gas_token_id, Bank, CallMessage as BankCallMessage, Coins};
use sov_evm::BANK_BALANCE_PRECOMPILE_ADDRESS;
use sov_evm::{
    AccountData, CallMessage, ContractCreationPolicy, EthereumAuthenticator, Evm, EvmChainSpec,
    EvmGenesisConfig, EvmRuntimeConfigUpdate, RlpEvmTransaction, SpecId,
};
use sov_evm_test_utils::PrecompileTester;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Amount, HexString, RawTx, SafeVec, Spec};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{
    AsUser, TestUser, TransactionTestCase, TransactionType, TEST_DEFAULT_USER_BALANCE,
};

/// Creates an EVM transaction that deploys a contract.
fn create_deploy_tx(nonce: u64, account: &EvmAccount, bytecode: Bytes) -> TxWithNonceAndHash {
    use sov_modules_api::macros::config_value;

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
        .sign_transaction(TypedTransaction::Eip1559(tx.clone()))
        .unwrap();
    let rlp = signed_tx.encoded_2718();
    let raw_tx = RawTx {
        data: borsh::to_vec(&RlpEvmTransaction { rlp }).unwrap(),
    };

    TxWithNonceAndHash {
        nonce: tx.nonce,
        hash: *signed_tx.hash(),
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
    }
}

/// Creates an EVM transaction that calls a contract.
fn create_contract_call_tx(
    nonce: u64,
    account: &EvmAccount,
    contract_address: Address,
    input: Bytes,
) -> TxWithNonceAndHash {
    use sov_modules_api::macros::config_value;

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
        .sign_transaction(TypedTransaction::Eip1559(tx.clone()))
        .unwrap();
    let rlp = signed_tx.encoded_2718();
    let raw_tx = RawTx {
        data: borsh::to_vec(&RlpEvmTransaction { rlp }).unwrap(),
    };

    TxWithNonceAndHash {
        nonce: tx.nonce,
        hash: *signed_tx.hash(),
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
    }
}

/// Sets up a test environment without the bank precompile enabled.
fn setup_without_precompile() -> (TestRunner<RT, S>, TestUser<S>, EvmAccount) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let evm_account = EvmAccount::generate();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData {
            address: evm_account.address(),
            code_hash: KECCAK_EMPTY,
            code: Default::default(),
        }],
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::CANCUN)],
        },
        contract_creation_policy: ContractCreationPolicy::Everyone,
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: admin.address(),
        enabled_precompiles: vec![], // No precompiles enabled
    };

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    // Add gas balance for the EVM account
    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (runner, admin, evm_account)
}

/// Sets up a test environment with the bank precompile enabled at genesis.
fn setup_with_precompile_at_genesis() -> (TestRunner<RT, S>, TestUser<S>, EvmAccount) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let evm_account = EvmAccount::generate();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData {
            address: evm_account.address(),
            code_hash: KECCAK_EMPTY,
            code: Default::default(),
        }],
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::CANCUN)],
        },
        contract_creation_policy: ContractCreationPolicy::Everyone,
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: admin.address(),
        enabled_precompiles: vec![BANK_BALANCE_PRECOMPILE_ADDRESS], // Bank precompile enabled
    };

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    // Add gas balance for the EVM account
    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    (runner, admin, evm_account)
}

/// Test that calling the bank balance precompile returns wrong result when not enabled.
/// When the precompile is not enabled, calling it is treated as a call to an EOA,
/// which returns empty bytes - not the expected balance.
#[test]
fn test_precompile_returns_wrong_result_when_not_enabled() {
    let (mut runner, _admin, evm_account) = setup_without_precompile();

    // First verify the precompile is NOT enabled in state
    let enabled = runner.query_state(|state| {
        let evm = Evm::<S>::default();
        evm.get_enabled_precompiles(state).unwrap()
    });
    assert!(
        !enabled.contains(&BANK_BALANCE_PRECOMPILE_ADDRESS),
        "Bank precompile should NOT be enabled"
    );

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

    // Create input for precompile: 20-byte address to query
    let query_address = evm_account.address();
    let precompile_input = Bytes::copy_from_slice(query_address.as_slice());

    // Call assertPrecompileResult with empty expected output.
    // Disabled precompiles behave like EOAs: the call succeeds with empty return data.
    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: BANK_BALANCE_PRECOMPILE_ADDRESS,
        input: precompile_input.clone(),
        expectedOutput: Bytes::new(), // Empty - EOA behavior
    };
    let call_data = Bytes::from(call.abi_encode());
    let call_tx = create_contract_call_tx(1, &evm_account, tester_address, call_data);

    runner.execute_transaction(TransactionTestCase {
        input: call_tx.tx,
        assert: Box::new(move |ctx, _state| {
            // Disabled precompile returns empty data (EOA behavior), so assertPrecompileResult
            // with empty expected output should succeed
            assert!(
                ctx.tx_receipt.is_successful(),
                "assertPrecompileResult with empty output should succeed for disabled precompile"
            );
        }),
    });
}

/// Test that the bank balance precompile works when enabled at genesis.
#[test]
fn test_precompile_enabled_at_genesis() {
    let (runner, _admin, _evm_account) = setup_with_precompile_at_genesis();

    // Verify the precompile is enabled using query_state
    let enabled = runner.query_state(|state| {
        let evm = Evm::<S>::default();
        evm.get_enabled_precompiles(state).unwrap()
    });

    assert!(
        enabled.contains(&BANK_BALANCE_PRECOMPILE_ADDRESS),
        "Bank precompile should be enabled at genesis"
    );
}

/// Test that the bank balance precompile can be enabled via admin transaction.
#[test]
fn test_precompile_enabled_via_admin_tx() {
    let (mut runner, admin, _evm_account) = setup_without_precompile();

    let precompile_addr_hex =
        HexString::<[u8; 20]>::from_str(&format!("{:?}", BANK_BALANCE_PRECOMPILE_ADDRESS)).unwrap();

    // First verify the precompile is NOT enabled
    let enabled = runner.query_state(|state| {
        let evm = Evm::<S>::default();
        evm.get_enabled_precompiles(state).unwrap()
    });

    assert!(
        !enabled.contains(&BANK_BALANCE_PRECOMPILE_ADDRESS),
        "Bank precompile should NOT be enabled initially"
    );

    // Enable the precompile via admin transaction
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: None,
                new_admin: None,
                enable_precompiles: Some(SafeVec::try_from(vec![precompile_addr_hex]).unwrap()),
            },
        )),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful(), "Admin tx should succeed");
            let evm = Evm::<S>::default();
            let enabled = evm.get_enabled_precompiles(state).unwrap();
            assert!(
                enabled.contains(&BANK_BALANCE_PRECOMPILE_ADDRESS),
                "Bank precompile should be enabled after admin tx"
            );
        }),
    });
}

/// Test that the bank balance precompile returns the correct balance via smart contract assertion.
#[test]
fn test_bank_balance_precompile_returns_correct_balance() {
    let (mut runner, _admin, evm_account) = setup_with_precompile_at_genesis();

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

    // Get the actual balance from the bank module to compute the expected output
    let query_address = evm_account.address();
    let rollup_address: <S as Spec>::Address =
        <S as Spec>::Address::from_vm_address(EthereumAddress::from(query_address));

    let expected_balance = runner.query_state(|state| {
        Bank::<S>::default()
            .get_balance_of(&rollup_address, config_gas_token_id(), state)
            .unwrap_infallible()
            .unwrap()
    });

    // Create input for precompile: 20-byte address to query
    let precompile_input = Bytes::copy_from_slice(query_address.as_slice());

    // Expected output: 32-byte U256 balance (big-endian)
    let expected_balance_u256 = U256::from(expected_balance.0);
    let expected_output = Bytes::copy_from_slice(&expected_balance_u256.to_be_bytes::<32>());

    // Call assertPrecompileResult - this should succeed
    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: BANK_BALANCE_PRECOMPILE_ADDRESS,
        input: precompile_input,
        expectedOutput: expected_output,
    };
    let call_data = Bytes::from(call.abi_encode());
    let call_tx = create_contract_call_tx(1, &evm_account, tester_address, call_data);

    runner.execute_transaction(TransactionTestCase {
        input: call_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Precompile should return the correct balance"
            );
        }),
    });
}

/// Test that bank balance updates are reflected in the precompile.
///
/// This test:
/// 1. Deploys the tester contract
/// 2. Verifies initial balance via precompile
/// 3. Transfers tokens using a sovereign bank transaction
/// 4. Verifies the precompile returns the updated balance
#[test]
fn test_bank_balance_precompile_reflects_transfers() {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(2);

    let admin = genesis_config.additional_accounts()[0].clone();
    let recipient = genesis_config.additional_accounts()[1].clone();

    let evm_account = EvmAccount::generate();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData {
            address: evm_account.address(),
            code_hash: KECCAK_EMPTY,
            code: Default::default(),
        }],
        chain_spec: EvmChainSpec {
            limit_contract_code_size: None,
            coinbase: Address::ZERO,
            block_gas_limit: 1_000_000_000,
            tx_gas_limit: Some(30_000_000),
            hardforks: vec![(0, SpecId::CANCUN)],
        },
        contract_creation_policy: ContractCreationPolicy::Everyone,
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: admin.address(),
        enabled_precompiles: vec![BANK_BALANCE_PRECOMPILE_ADDRESS],
    };

    let mut genesis = GenesisConfig::from_minimal_config(genesis_config.into(), evm_config);

    // Add gas balance for the EVM account
    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let mut runner: TestRunner<RT, S> =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestRuntime::default());

    let recipient_address: <S as Spec>::Address = recipient.address();
    // Convert sovereign address to ETH address (take first 20 bytes)
    let recipient_eth_address = Address::from_slice(&recipient_address.as_ref()[..20]);

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

    // Get initial balance
    let initial_balance = runner.query_state(|state| {
        Bank::<S>::default()
            .get_balance_of(&recipient_address, config_gas_token_id(), state)
            .unwrap_infallible()
            .unwrap_or_default()
    });

    // Verify initial balance via precompile
    let precompile_input = Bytes::copy_from_slice(recipient_eth_address.as_slice());
    let initial_balance_u256 = U256::from(initial_balance.0);
    let initial_expected_output =
        Bytes::copy_from_slice(&initial_balance_u256.to_be_bytes::<32>());

    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: BANK_BALANCE_PRECOMPILE_ADDRESS,
        input: precompile_input.clone(),
        expectedOutput: initial_expected_output,
    };
    let call_data = Bytes::from(call.abi_encode());
    let call_tx = create_contract_call_tx(1, &evm_account, tester_address, call_data);

    runner.execute_transaction(TransactionTestCase {
        input: call_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Initial balance check should succeed"
            );
        }),
    });

    let transfer_amount = Amount::new(1000);

    // Transfer tokens to recipient using admin account (sovereign tx, not EVM tx)
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Bank<S>>(BankCallMessage::Transfer {
            to: recipient_address,
            coins: Coins {
                amount: transfer_amount,
                token_id: config_gas_token_id(),
            },
        }),
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Bank transfer should succeed"
            );
        }),
    });

    // Verify updated balance via precompile
    let new_balance = initial_balance.checked_add(transfer_amount).unwrap();
    let new_balance_u256 = U256::from(new_balance.0);
    let new_expected_output = Bytes::copy_from_slice(&new_balance_u256.to_be_bytes::<32>());

    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: BANK_BALANCE_PRECOMPILE_ADDRESS,
        input: precompile_input,
        expectedOutput: new_expected_output,
    };
    let call_data = Bytes::from(call.abi_encode());
    let call_tx = create_contract_call_tx(2, &evm_account, tester_address, call_data);

    runner.execute_transaction(TransactionTestCase {
        input: call_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Precompile should return updated balance after transfer"
            );
        }),
    });
}

/// Test that enabling a precompile via admin tx makes it work.
/// This combines the admin enable test with actual precompile usage.
#[test]
fn test_precompile_works_after_admin_enables_it() {
    let (mut runner, admin, evm_account) = setup_without_precompile();

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

    // Set up address for precompile call
    let query_address = evm_account.address();
    let rollup_address: <S as Spec>::Address =
        <S as Spec>::Address::from_vm_address(EthereumAddress::from(query_address));
    let precompile_input = Bytes::copy_from_slice(query_address.as_slice());

    // First, verify the precompile returns empty data (disabled = EOA behavior)
    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: BANK_BALANCE_PRECOMPILE_ADDRESS,
        input: precompile_input.clone(),
        expectedOutput: Bytes::new(), // Empty - EOA behavior for disabled precompile
    };
    let call_data = Bytes::from(call.abi_encode());
    let call_tx = create_contract_call_tx(1, &evm_account, tester_address, call_data);

    runner.execute_transaction(TransactionTestCase {
        input: call_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Disabled precompile should return empty data (EOA behavior)"
            );
        }),
    });

    // Enable the precompile via admin transaction
    let precompile_addr_hex =
        HexString::<[u8; 20]>::from_str(&format!("{:?}", BANK_BALANCE_PRECOMPILE_ADDRESS)).unwrap();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Evm<S>>(CallMessage::UpdateRuntimeConfig(
            EvmRuntimeConfigUpdate {
                new_hardfork: None,
                new_contract_creation_policy: None,
                chain_spec_update: None,
                new_admin: None,
                enable_precompiles: Some(SafeVec::try_from(vec![precompile_addr_hex]).unwrap()),
            },
        )),
        assert: Box::new(move |ctx, _state| {
            assert!(ctx.tx_receipt.is_successful(), "Admin tx should succeed");
        }),
    });

    // Now the precompile call should succeed
    // Need to re-query the balance since gas was spent
    let expected_balance = runner.query_state(|state| {
        Bank::<S>::default()
            .get_balance_of(&rollup_address, config_gas_token_id(), state)
            .unwrap_infallible()
            .unwrap()
    });
    let expected_balance_u256 = U256::from(expected_balance.0);
    let expected_output = Bytes::copy_from_slice(&expected_balance_u256.to_be_bytes::<32>());

    let call = PrecompileTester::assertPrecompileResultCall {
        precompile: BANK_BALANCE_PRECOMPILE_ADDRESS,
        input: precompile_input,
        expectedOutput: expected_output,
    };
    let call_data = Bytes::from(call.abi_encode());
    let call_tx = create_contract_call_tx(2, &evm_account, tester_address, call_data);

    runner.execute_transaction(TransactionTestCase {
        input: call_tx.tx,
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Precompile call should succeed after enabling"
            );
        }),
    });
}
