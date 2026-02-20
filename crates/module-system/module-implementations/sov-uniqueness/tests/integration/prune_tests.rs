use std::str::FromStr;

use alloy_consensus::constants::KECCAK_EMPTY;
use sov_address::{EthereumAddress, FromVmAddress, MultiAddress};
use sov_evm::{AccountData, EvmChainSpec, EvmGenesisConfig, SpecId};
use sov_modules_api::capabilities::{config_chain_id, TransactionAuthenticator, UniquenessData};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::transaction::{Transaction, UnsignedTransaction};
use sov_modules_api::{CredentialId, EncodeCall, HexHash, RawTx};
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::{Runtime, TestRunner, ValueSetterConfig};
use sov_test_utils::{
    TestUser, TransactionTestCase, TransactionType, TEST_DEFAULT_MAX_FEE,
    TEST_DEFAULT_MAX_PRIORITY_FEE, TEST_DEFAULT_USER_BALANCE,
};
use sov_uniqueness::Uniqueness;

use crate::runtime::{GenesisConfig, TestNonceRuntime, RT, S};
use crate::utils::{generate_value_setter_tx, EvmAccount};

/// Creates a test setup where the chain_state admin is set to the first additional account.
fn setup_with_chain_admin() -> (TestUser<S>, TestUser<S>, TestRunner<TestNonceRuntime<S>, S>) {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(2);

    let chain_admin = genesis_config.additional_accounts()[0].clone();
    let non_admin = genesis_config.additional_accounts()[1].clone();

    let evm_account = EvmAccount::generate();

    let evm_config = EvmGenesisConfig {
        accounts: vec![AccountData {
            address: evm_account.address(),
            code_hash: KECCAK_EMPTY,
            code: Default::default(),
        }],
        chain_spec: EvmChainSpec {
            hardforks: vec![(0, SpecId::CANCUN)].into_iter().collect(),
            ..Default::default()
        },
        contract_creation_policy: Default::default(),
        initial_base_fee: 0,
        genesis_timestamp: 0,
        admin: MultiAddress::from_vm_address(
            EthereumAddress::from_str("0x0123456789012345678901234567890123456789").unwrap(),
        ),
    };

    let mut genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        ValueSetterConfig {
            admin: chain_admin.address(),
        },
        evm_config,
    );

    // Set the chain_state admin to our first additional account.
    genesis.chain_state.admin = Some(chain_admin.address());

    if let Some(c) = genesis.bank.gas_token_config.as_mut() {
        c.address_and_balances.push((
            MultiAddress::Vm(EthereumAddress::from(evm_account.address())),
            TEST_DEFAULT_USER_BALANCE,
        ));
    }

    let runner =
        TestRunner::new_with_genesis(genesis.into_genesis_params(), TestNonceRuntime::default());

    (chain_admin, non_admin, runner)
}

/// Builds a transaction that calls `Uniqueness::PruneSelfGenerations` signed by `sender`.
fn generate_prune_self_tx(
    credential_id: CredentialId,
    generation: u64,
    sender: &TestUser<S>,
) -> TransactionType<RT, S> {
    let runtime_msg = <RT as EncodeCall<Uniqueness<S>>>::to_decodable(
        sov_uniqueness::CallMessage::PruneSelfGenerations { credential_id },
    );

    let transaction = UnsignedTransaction::new(
        runtime_msg,
        config_chain_id(),
        TEST_DEFAULT_MAX_PRIORITY_FEE,
        TEST_DEFAULT_MAX_FEE,
        UniquenessData::Generation(generation),
        None,
    );

    let transaction = Transaction::<RT, S>::new_signed_tx(
        sender.private_key(),
        &<TestNonceRuntime<S> as Runtime<S>>::CHAIN_HASH,
        transaction,
    );
    TransactionType::PreAuthenticated(<RT as Runtime<S>>::Auth::encode_with_standard_auth(RawTx {
        data: borsh::to_vec(&transaction).unwrap(),
    }))
}

/// Builds a transaction that calls `Uniqueness::PruneGenerations` signed by `sender`.
fn generate_prune_tx(
    credential_ids: Vec<CredentialId>,
    generation: u64,
    sender: &TestUser<S>,
) -> TransactionType<RT, S> {
    let runtime_msg = <RT as EncodeCall<Uniqueness<S>>>::to_decodable(
        sov_uniqueness::CallMessage::PruneGenerations { credential_ids },
    );

    let transaction = UnsignedTransaction::new(
        runtime_msg,
        config_chain_id(),
        TEST_DEFAULT_MAX_PRIORITY_FEE,
        TEST_DEFAULT_MAX_FEE,
        UniquenessData::Generation(generation),
        None,
    );

    let transaction = Transaction::<RT, S>::new_signed_tx(
        sender.private_key(),
        &<TestNonceRuntime<S> as Runtime<S>>::CHAIN_HASH,
        transaction,
    );
    TransactionType::PreAuthenticated(<RT as Runtime<S>>::Auth::encode_with_standard_auth(RawTx {
        data: borsh::to_vec(&transaction).unwrap(),
    }))
}

#[test]
fn test_prune_generations_by_admin_succeeds() {
    let (chain_admin, _, mut runner) = setup_with_chain_admin();
    let admin_credential_id: CredentialId = chain_admin.credential_id();

    // First, create some generation data by sending a value setter tx.
    runner.execute_transaction(TransactionTestCase {
        input: generate_value_setter_tx(0, 42, &chain_admin),
        assert: Box::new(|ctx, _| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Initial value setter tx should succeed"
            );
        }),
    });

    // Verify generation data exists.
    runner.query_visible_state(|state| {
        assert_eq!(
            Uniqueness::<S>::default()
                .next_generation(&admin_credential_id, state)
                .unwrap(),
            1,
            "Generation data should exist after sending a transaction"
        );
    });

    // Prune the generation data as chain_state admin.
    runner.execute_transaction(TransactionTestCase {
        input: generate_prune_tx(vec![admin_credential_id], 1, &chain_admin),
        assert: Box::new(move |ctx, state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Pruning by admin should succeed"
            );

            // After pruning, next_generation should return 0 (no data).
            assert_eq!(
                Uniqueness::<S>::default()
                    .next_generation(&admin_credential_id, state)
                    .unwrap(),
                0,
                "Generation data should be cleared after pruning"
            );
        }),
    });
}

#[test]
fn test_prune_generations_by_non_admin_fails() {
    let (chain_admin, non_admin, mut runner) = setup_with_chain_admin();
    let admin_credential_id: CredentialId = chain_admin.credential_id();

    // Create some generation data.
    runner.execute_transaction(TransactionTestCase {
        input: generate_value_setter_tx(0, 42, &chain_admin),
        assert: Box::new(|ctx, _| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Initial value setter tx should succeed"
            );
        }),
    });

    // Try to prune as non-admin — should fail.
    runner.execute_transaction(TransactionTestCase {
        input: generate_prune_tx(vec![admin_credential_id], 0, &non_admin),
        assert: Box::new(move |ctx, state| {
            assert!(
                !ctx.tx_receipt.is_successful(),
                "Pruning by non-admin should fail"
            );

            // Generation data should still exist.
            assert_eq!(
                Uniqueness::<S>::default()
                    .next_generation(&admin_credential_id, state)
                    .unwrap(),
                1,
                "Generation data should remain intact after failed prune attempt"
            );
        }),
    });
}

#[test]
fn test_prune_does_not_affect_nonces() {
    let (chain_admin, _, mut runner) = setup_with_chain_admin();
    let admin_credential_id: CredentialId = chain_admin.credential_id();

    // Create both generation and nonce data. Generation data comes from
    // standard value setter txs (which use generation-based dedup).
    runner.execute_transaction(TransactionTestCase {
        input: generate_value_setter_tx(0, 42, &chain_admin),
        assert: Box::new(|ctx, _| {
            assert!(ctx.tx_receipt.is_successful());
        }),
    });

    // Verify both generation and nonce-like state exist.
    runner.query_visible_state(|state| {
        let uniqueness = Uniqueness::<S>::default();
        assert_eq!(
            uniqueness
                .next_generation(&admin_credential_id, state)
                .unwrap(),
            1,
            "Generation data should exist"
        );
        // Nonces are tracked separately — the nonce should be None since
        // our test tx uses generation-based dedup, not nonce-based.
        assert_eq!(
            uniqueness
                .nonce(&admin_credential_id, state)
                .unwrap_infallible(),
            None,
            "Nonce should not be set for generation-based transactions"
        );
    });

    // Prune generations.
    runner.execute_transaction(TransactionTestCase {
        input: generate_prune_tx(vec![admin_credential_id], 1, &chain_admin),
        assert: Box::new(move |ctx, state| {
            assert!(ctx.tx_receipt.is_successful(), "Pruning should succeed");

            let uniqueness = Uniqueness::<S>::default();
            // Generation data should be gone.
            assert_eq!(
                uniqueness
                    .next_generation(&admin_credential_id, state)
                    .unwrap(),
                0,
                "Generation data should be cleared"
            );
            // Nonce should be unaffected.
            assert_eq!(
                uniqueness
                    .nonce(&admin_credential_id, state)
                    .unwrap_infallible(),
                None,
                "Nonce should remain unaffected by prune"
            );
        }),
    });
}

#[test]
fn test_prune_nonexistent_credential_is_noop() {
    let (chain_admin, _, mut runner) = setup_with_chain_admin();

    // Create a credential ID that has no generation data.
    let nonexistent_credential_id = CredentialId(HexHash::new([99; 32]));

    // Pruning a nonexistent credential should succeed silently.
    runner.execute_transaction(TransactionTestCase {
        input: generate_prune_tx(vec![nonexistent_credential_id], 0, &chain_admin),
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Pruning a nonexistent credential should succeed as a no-op"
            );
        }),
    });
}

#[test]
fn test_prune_self_generations_succeeds() {
    let (chain_admin, _, mut runner) = setup_with_chain_admin();
    let admin_credential_id: CredentialId = chain_admin.credential_id();

    // Create some generation data by sending a value setter tx.
    runner.execute_transaction(TransactionTestCase {
        input: generate_value_setter_tx(0, 42, &chain_admin),
        assert: Box::new(|ctx, _| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Initial value setter tx should succeed"
            );
        }),
    });

    // Verify generation data exists.
    runner.query_visible_state(|state| {
        assert_eq!(
            Uniqueness::<S>::default()
                .next_generation(&admin_credential_id, state)
                .unwrap(),
            1,
            "Generation data should exist after sending a transaction"
        );
    });

    // Self-prune the caller's own generation data.
    runner.execute_transaction(TransactionTestCase {
        input: generate_prune_self_tx(admin_credential_id, 1, &chain_admin),
        assert: Box::new(move |ctx, state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Self-pruning should succeed"
            );

            assert_eq!(
                Uniqueness::<S>::default()
                    .next_generation(&admin_credential_id, state)
                    .unwrap(),
                0,
                "Generation data should be cleared after self-pruning"
            );
        }),
    });
}

#[test]
fn test_prune_self_wrong_credential_fails() {
    let (chain_admin, non_admin, mut runner) = setup_with_chain_admin();
    let admin_credential_id: CredentialId = chain_admin.credential_id();

    // Create some generation data for the admin.
    runner.execute_transaction(TransactionTestCase {
        input: generate_value_setter_tx(0, 42, &chain_admin),
        assert: Box::new(|ctx, _| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Initial value setter tx should succeed"
            );
        }),
    });

    // Non-admin tries to self-prune the admin's credential — should fail
    // because the credential doesn't derive to the non-admin's address.
    runner.execute_transaction(TransactionTestCase {
        input: generate_prune_self_tx(admin_credential_id, 0, &non_admin),
        assert: Box::new(move |ctx, state| {
            assert!(
                !ctx.tx_receipt.is_successful(),
                "Self-pruning another user's credential should fail"
            );

            // Admin's generation data should still exist.
            assert_eq!(
                Uniqueness::<S>::default()
                    .next_generation(&admin_credential_id, state)
                    .unwrap(),
                1,
                "Generation data should remain intact after failed self-prune"
            );
        }),
    });
}

#[test]
fn test_prune_self_by_non_admin_succeeds() {
    let (_chain_admin, non_admin, mut runner) = setup_with_chain_admin();
    let non_admin_credential_id: CredentialId = non_admin.credential_id();

    // Non-admin self-prunes their own credential — no admin check needed.
    // There's no pre-existing generation data, so this is a no-op delete,
    // but the important thing is that the call succeeds without admin auth.
    runner.execute_transaction(TransactionTestCase {
        input: generate_prune_self_tx(non_admin_credential_id, 0, &non_admin),
        assert: Box::new(move |ctx, _state| {
            assert!(
                ctx.tx_receipt.is_successful(),
                "Non-admin self-pruning their own credential should succeed"
            );
        }),
    });
}
