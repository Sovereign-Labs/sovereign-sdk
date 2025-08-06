//! Tests for gasless "setup mode" transactions
//!
//! These require special testing in the sequencer because they rely on support from the blob selector.

use std::future::Future;

use crate::preferred_end_to_end::tx_set_value;
use crate::utils::new_test_rollup;
use crate::utils::tempdir_inside_codebase_dir;
use crate::utils::MAX_BATCH_EXECUTION_TIME_MILLIS;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use sov_api_spec::types::{self as api_types};
use sov_mock_da::BlockProducingConfig;
use sov_mock_zkvm::crypto::private_key::Ed25519PrivateKey;
use sov_modules_api::macros::config_value;
use sov_modules_api::transaction::PriorityFeeBips;
use sov_modules_api::transaction::TxDetails;
use sov_modules_api::Amount;
use sov_modules_api::DaSpec;
use sov_modules_api::DispatchCall;
use sov_modules_api::RawTx;
use sov_modules_api::Spec;
use sov_modules_stf_blueprint::GenesisParams;
use sov_modules_stf_blueprint::Runtime;
use sov_test_utils::default_test_signed_transaction;
use sov_test_utils::generate_optimistic_runtime_with_kernel;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::Coins;
use sov_test_utils::runtime::TokenId;
use sov_test_utils::test_rollup::TestRollup;
use sov_test_utils::test_signed_transaction;
use sov_test_utils::RtAgnosticBlueprint;
use sov_test_utils::TestSpec;
use sov_test_utils::TEST_BLOB_PROCESSING_TIMEOUT;
use sov_test_utils::TEST_FINALIZATION_BLOCKS;
use sov_test_utils::TEST_MAX_BATCH_SIZE;
use sov_test_utils::{sov_bank, sov_sequencer_registry};
use sov_value_setter::{ValueSetter, ValueSetterConfig};

use crate::preferred_end_to_end::DaLayerWithSubscription;

pub(crate) type TestBlueprint = RtAgnosticBlueprint<TestSpec, TestRuntime<TestSpec>>;
const TEST_SETUP_MODE_DISABLE_AT_SLOT: u64 = 10;

generate_optimistic_runtime_with_kernel!(
    TestRuntime <=
    kernel_type: sov_kernels::soft_confirmations::SoftConfirmationsKernel<'a, S>,
    modules: [value_setter: ValueSetter<S>],
    transaction_delay_ms_wrapper: |_: &Self::Decodable| {
        0
    }
);

#[tokio::test(flavor = "multi_thread")]
async fn test_gasless_setup_mode_with_explicit_disable() {
    let disable_setup_mode_fn = |rollup: TestRollup<TestBlueprint>,
                                 mut da_layer: DaLayerWithSubscription,
                                 admin_key: Ed25519PrivateKey| async move {
        // Send the call to disable setup mode. Should succeed.
        {
            let tx = encode_call(&admin_key, 0, &remove_setup_mode_call());
            rollup
                .client
                .client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx),
                })
                .await
                .expect("Transaction should succeed");
        }

        // Kick off the next slot.
        rollup.force_close_batch().await.unwrap();
        da_layer.produce_and_wait_for_n_slots(1).await;
        rollup
    };
    test_gasless_setup_mode(disable_setup_mode_fn).await;
}

/// Test the setup mode without sending any transactions to disable it. It gets disabled automatically after a few slots
#[tokio::test(flavor = "multi_thread")]
async fn test_gasless_setup_mode_with_implicit_disable() {
    let disable_setup_mode_fn = |rollup: TestRollup<TestBlueprint>,
                                 mut da_layer: DaLayerWithSubscription,
                                 admin_key: Ed25519PrivateKey| async move {
        for i in 1..=TEST_SETUP_MODE_DISABLE_AT_SLOT {
            // Kick off the next slot. Send a tx to ensure that a batch is in progress before force closing.
            let tx = encode_call(&admin_key, i, &value_setter_call(i as u32));
            rollup
                .client
                .client
                .accept_tx(&api_types::AcceptTxBody {
                    body: BASE64_STANDARD.encode(&tx),
                })
                .await
                .expect("Transaction should succeed");
            rollup.force_close_batch().await.unwrap();
            da_layer.produce_and_wait_for_n_slots(1).await;
        }
        rollup
    };
    test_gasless_setup_mode(disable_setup_mode_fn).await;
}

async fn test_gasless_setup_mode<F, Fut>(disable_fn: F)
where
    // Note: We pass all the arguments by value because rustc's lifetime analysis on async functions isn't very smart
    F: FnOnce(TestRollup<TestBlueprint>, DaLayerWithSubscription, Ed25519PrivateKey) -> Fut,
    Fut: Future<Output = TestRollup<TestBlueprint>>,
{
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_SETUP_MODE_TERMINATION_HEIGHT",
        TEST_SETUP_MODE_DISABLE_AT_SLOT.to_string(),
    );

    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);
    let preferred_sequencer = genesis_config.initial_sequencer.clone();
    let admin = genesis_config.additional_accounts()[0].clone();
    let default_balance = admin.balance();

    let mut rt_genesis_config =
        <TestRuntime<TestSpec> as Runtime<TestSpec>>::GenesisConfig::from_minimal_config(
            genesis_config.into(),
            ValueSetterConfig {
                admin: admin.address(),
            },
        );

    rt_genesis_config.chain_state.admin = Some(admin.address());
    rt_genesis_config
        .bank
        .gas_token_config
        .address_and_balances
        .clear();
    rt_genesis_config.bank.gas_token_config.admins = vec![admin.address()];
    rt_genesis_config
        .sequencer_registry
        .sequencer_config
        .seq_bond = Amount::ZERO;
    rt_genesis_config
        .attester_incentives
        .initial_attesters
        .iter_mut()
        .for_each(|(_addr, bond)| {
            *bond = Amount::ZERO;
        });
    rt_genesis_config
        .prover_incentives
        .initial_provers
        .iter_mut()
        .for_each(|(_addr, bond)| {
            *bond = Amount::ZERO;
        });

    let genesis_params = GenesisParams {
        runtime: rt_genesis_config.clone(),
    };

    let dir = tempdir_inside_codebase_dir();

    let rollup = new_test_rollup::<TestRuntime<TestSpec>>(
        dir.clone(),
        genesis_params
            .runtime
            .sequencer_registry
            .sequencer_config
            .seq_da_address,
        genesis_params,
        0,
        true,
        TEST_MAX_BATCH_SIZE,
        BlockProducingConfig::Manual,
        None,
        TEST_BLOB_PROCESSING_TIMEOUT,
        1,
        MAX_BATCH_EXECUTION_TIME_MILLIS,
        None,
        TEST_FINALIZATION_BLOCKS,
    )
    .await
    .map(|v| v.into_iter().next().unwrap());

    let Some(test_rollup) = rollup else {
        return;
    };

    // Produce a few blocks to DA blocks to make sure there's a finalized slot after genesis.
    let mut da_layer = DaLayerWithSubscription::new(&test_rollup).await;
    da_layer.produce_and_wait_for_n_slots(5).await;

    let client = test_rollup.api_client().clone();
    // Send a transaction with a non-zero fee. Should fail, because we have no balance.
    {
        let tx_with_fee = tx_set_value(&admin.private_key, 0, 7);
        let err = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx_with_fee),
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("Insufficient balance to pay for the transaction gas"),
            "Expected Out of gas error. Got: {err}"
        );
    }

    // Send a transaction with a zero fee. Should succeed
    {
        let tx = encode_zero_gas_tx(&admin.private_key, 0, &value_setter_call(7));
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .expect("Transaction should succeed");
    }

    // Mint some tokens to the admin. Should succeed.
    {
        let tx = encode_zero_gas_tx(
            &admin.private_key,
            0,
            &mint_gas_token_call(default_balance, admin.address()),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .expect("Transaction should succeed");
    }

    // Bond some tokens for the preferred sequencer
    {
        let tx = encode_call(
            &admin.private_key,
            0,
            &mint_gas_token_call(
                default_balance.saturating_mul(2u64.into()),
                preferred_sequencer.user_info.address(),
            ),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .expect("Transaction should succeed");

        let tx = encode_call(
            &preferred_sequencer.user_info.private_key,
            0,
            &deposit_call(default_balance, preferred_sequencer.da_address),
        );
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .expect("Transaction should succeed");
    }

    let test_rollup = disable_fn(test_rollup, da_layer, admin.private_key().clone()).await;

    // Send a transaction with a zero fee. Should fail because setup mode is now disabled.
    {
        let tx = encode_zero_gas_tx(&admin.private_key, 100, &value_setter_call(7));
        let err = client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("The amount to charge is greater than the funds available in the meter"),
            "Expected Out of gas error. Got: {err}"
        );
    }

    // Send another transaction with a non-zero fee. Should succeed.
    {
        let tx = encode_call(&admin.private_key, 100, &value_setter_call(7));
        client
            .accept_tx(&api_types::AcceptTxBody {
                body: BASE64_STANDARD.encode(&tx),
            })
            .await
            .expect("Transaction should succeed");
    }

    test_rollup.shutdown().await.unwrap();
}

fn value_setter_call(value_to_set: u32) -> <TestRuntime<TestSpec> as DispatchCall>::Decodable {
    <TestRuntime<TestSpec> as DispatchCall>::Decodable::ValueSetter(
        sov_value_setter::CallMessage::SetValue {
            value: value_to_set,
            gas: None,
        },
    )
}

fn remove_setup_mode_call() -> <TestRuntime<TestSpec> as DispatchCall>::Decodable {
    <TestRuntime<TestSpec> as DispatchCall>::Decodable::ChainState(
        sov_chain_state::CallMessage::TerminateSetupMode,
    )
}

fn deposit_call(
    amount: Amount,
    da_address: <<TestSpec as Spec>::Da as DaSpec>::Address,
) -> <TestRuntime<TestSpec> as DispatchCall>::Decodable {
    <TestRuntime<TestSpec> as DispatchCall>::Decodable::SequencerRegistry(
        sov_sequencer_registry::CallMessage::Deposit { da_address, amount },
    )
}

fn mint_gas_token_call(
    amount: Amount,
    recipient: <TestSpec as Spec>::Address,
) -> <TestRuntime<TestSpec> as DispatchCall>::Decodable {
    <TestRuntime<TestSpec> as DispatchCall>::Decodable::Bank(sov_bank::CallMessage::Mint {
        coins: Coins {
            token_id: config_value!("GAS_TOKEN_ID"),
            amount,
        },
        mint_to_address: recipient,
    })
}

fn encode_zero_gas_tx(
    key: &Ed25519PrivateKey,
    nonce: u64,
    call_message: &<TestRuntime<TestSpec> as DispatchCall>::Decodable,
) -> RawTx {
    let details = TxDetails {
        max_fee: Amount::ZERO,
        max_priority_fee_bips: PriorityFeeBips::ZERO,
        gas_limit: None,
        chain_id: config_value!("CHAIN_ID"),
    };
    let tx = test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        key,
        call_message,
        nonce,
        &<TestRuntime<TestSpec> as Runtime<TestSpec>>::CHAIN_HASH,
        details,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}

fn encode_call(
    key: &Ed25519PrivateKey,
    nonce: u64,
    call_message: &<TestRuntime<TestSpec> as DispatchCall>::Decodable,
) -> RawTx {
    let tx = default_test_signed_transaction::<TestRuntime<TestSpec>, TestSpec>(
        key,
        call_message,
        nonce,
        &<TestRuntime<TestSpec> as Runtime<TestSpec>>::CHAIN_HASH,
    );

    RawTx::new(borsh::to_vec(&tx).unwrap())
}
