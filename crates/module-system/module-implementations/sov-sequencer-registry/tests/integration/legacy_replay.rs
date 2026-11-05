//! Historical roots and receipts captured with this scenario on dev commit
//! 509a482949d553e3b178cac77290d9ecea15d9f2. The fixture must not be regenerated
//! from the upgraded implementation: check out that commit, copy this file, the fixture and
//! the `mod legacy_replay;` line into it, run `replay` there and copy the output back.
//! `PINNED_ROOTS` fails the test if the fixture is regenerated from code that changed
//! pre-activation gas.

use std::collections::HashMap;

use sov_bank::{Bank, IntoPayable};
use sov_mock_da::MockAddress;
use sov_modules_api::capabilities::HasKernel;
use sov_modules_api::prelude::serde_json::{self, json, Value};
use sov_modules_api::{
    Amount, CryptoSpec, Gas, HexHash, ModuleInfo, Spec, StateCheckpoint, TxEffect,
};
use sov_sequencer_registry::{CallMessage, SequencerRegistry};
use sov_test_utils::runtime::genesis::optimistic::{
    HighLevelOptimisticGenesisConfig, MinimalOptimisticGenesisConfig,
};
use sov_test_utils::runtime::{TestRunner, ValueSetterConfig};
use sov_test_utils::storage::ForklessStorageManager;
use sov_test_utils::{
    get_gas_used, AsUser, TestAttester, TestChallenger, TestSequencer, TestUser, TransactionType,
    TEST_DEFAULT_USER_BALANCE,
};

use crate::helpers::{GenesisConfig, RT, S};

const BOND: Amount = Amount::new(100_000_000_000);
const BASELINE: &str = include_str!("legacy_replay.json");
const PINNED_ROOTS: [(&str, &str); 2] = [
    (
        "register",
        "5d618ac4865ed6c968320cc5a69b5d1de26247a26f4b6a195628571abe5a06bd650e53fea92aae89aa5a1aca32f560a5bfff85ecc0b737886a0e7c007edfbd29",
    ),
    (
        "withdraw",
        "3ebbdc1a19034fba846278507155894c658f2cd21797a15ff7ca258b3dfd5493117abc924dfe011d57b66c0575a4b8fbd9cb673ec4067551cf27025ed9062f4f",
    ),
];

fn user(seed: u8) -> TestUser<S> {
    type Key = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey;
    TestUser::new(
        Key::try_from(vec![seed; 32]).unwrap(),
        TEST_DEFAULT_USER_BALANCE,
    )
}

fn runner() -> (TestRunner<RT, S>, TestUser<S>, TestUser<S>) {
    let sequencer = user(1);
    let owner = user(2);
    let stranger = user(3);
    let genesis = HighLevelOptimisticGenesisConfig::with_defaults(
        TestAttester {
            user_info: sequencer.clone(),
            bond: BOND,
            slot_to_attest: 1,
        },
        TestChallenger {
            user_info: sequencer.clone(),
        },
        TestSequencer {
            user_info: sequencer,
            da_address: MockAddress::new([0; 32]),
            bond: BOND,
        },
        vec![owner.clone(), stranger.clone()],
        Default::default(),
        Default::default(),
    );
    let mut genesis: MinimalOptimisticGenesisConfig<S> = genesis.into();
    // The high-level helper inserts a randomly funded placeholder prover even in optimistic mode.
    let placeholder = user(4).address();
    let random_placeholder = genesis
        .config
        .prover_incentives
        .admin
        .replace(placeholder)
        .unwrap();
    genesis.config.prover_incentives.initial_provers[0].0 = placeholder;
    for (address, _) in &mut genesis
        .config
        .bank
        .gas_token_config
        .as_mut()
        .unwrap()
        .address_and_balances
    {
        if *address == random_placeholder {
            *address = placeholder;
        }
    }
    let genesis = GenesisConfig::from_minimal_config(
        genesis,
        ValueSetterConfig {
            admin: owner.address(),
        },
    );
    let mut runner = TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default());
    runner.config.freeze_time = Some(sov_modules_api::da::Time::from_secs(1_000));
    (runner, owner, stranger)
}

fn snapshot(runner: &TestRunner<RT, S>, name: &str, receipt: Value) -> Value {
    runner.query_visible_state(|state| {
        let registry = SequencerRegistry::<S>::default();
        let balances = [1, 2, 3, 4]
            .map(|seed| TestRunner::<RT, S>::bank_gas_balance(&user(seed).address(), state));
        let registrations = [0, 10, 11].map(|seed| {
            registry
                .is_sender_known(&MockAddress::new([seed; 32]), state)
                .ok()
        });
        json!({
            "name": name,
            "root": runner.state_root().to_string(),
            "receipt": receipt,
            "balances": balances,
            "registrations": registrations,
        })
    })
}

fn replay(chain_hash: [u8; 32]) -> Value {
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_SEQUENCER_DA_ROTATION_ACTIVATION_HEIGHT",
        "1000",
    );
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_DEFERRED_SLOTS_COUNT", "3");
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_CHAIN_HASH_OVERRIDES",
        format!(
            r#"[{{ start_height = 0, end_height = 1000, chain_hash = "{}" }}]"#,
            HexHash::new(chain_hash)
        ),
    );
    let (mut runner, owner, stranger) = runner();
    let mut nonces = HashMap::new();
    let mut blocks = vec![snapshot(&runner, "genesis", Value::Null)];
    let da_a = MockAddress::new([10; 32]);
    let da_b = MockAddress::new([11; 32]);
    let cases = [
        (
            "register",
            &owner,
            CallMessage::Register {
                da_address: da_a,
                amount: BOND,
            },
        ),
        (
            "register_second_da_same_owner",
            &owner,
            CallMessage::Register {
                da_address: da_b,
                amount: BOND,
            },
        ),
        (
            "register_duplicate",
            &owner,
            CallMessage::Register {
                da_address: da_a,
                amount: BOND,
            },
        ),
        (
            "deposit",
            &owner,
            CallMessage::Deposit {
                da_address: da_a,
                amount: Amount::new(1234),
            },
        ),
        (
            "deposit_wrong_owner",
            &stranger,
            CallMessage::Deposit {
                da_address: da_a,
                amount: Amount::new(1234),
            },
        ),
        (
            "withdraw_not_initiated",
            &owner,
            CallMessage::Withdraw { da_address: da_a },
        ),
        (
            "initiate_withdrawal",
            &owner,
            CallMessage::InitiateWithdrawal { da_address: da_a },
        ),
        (
            "withdraw_too_early",
            &owner,
            CallMessage::Withdraw { da_address: da_a },
        ),
        (
            "initiate_twice",
            &owner,
            CallMessage::InitiateWithdrawal { da_address: da_a },
        ),
    ];
    for (name, signer, message) in cases {
        let TransactionType::Plain {
            message,
            key,
            details,
        } = signer.create_plain_message::<RT, SequencerRegistry<S>>(message)
        else {
            unreachable!()
        };
        let input =
            TransactionType::<RT, S>::PreSigned(TransactionType::<RT, S>::sign_and_serialize(
                message,
                key,
                &chain_hash,
                details,
                &mut nonces,
            ));
        let (result, _) = runner.execute(input);
        let tx = result.last_tx_receipt();
        if matches!(
            name,
            "register" | "register_second_da_same_owner" | "deposit" | "initiate_withdrawal"
        ) {
            assert!(tx.receipt.is_successful(), "{name}: {:?}", tx.receipt);
        } else {
            assert!(
                matches!(tx.receipt, TxEffect::Reverted(_)),
                "{name}: {:?}",
                tx.receipt
            );
        }
        let gas = get_gas_used(tx);
        let receipt = json!({ "tx": tx, "gas": gas, "gas_value": gas.value(result.last_batch_receipt().inner.gas_price) });
        blocks.push(snapshot(&runner, name, receipt));
    }
    for _ in 0..4 {
        runner.advance_slots(1);
        blocks.push(snapshot(&runner, "wait_for_withdrawal", Value::Null));
    }
    let TransactionType::Plain {
        message,
        key,
        details,
    } = owner.create_plain_message::<RT, SequencerRegistry<S>>(CallMessage::Withdraw {
        da_address: da_a,
    })
    else {
        unreachable!()
    };
    let input = TransactionType::<RT, S>::PreSigned(TransactionType::<RT, S>::sign_and_serialize(
        message,
        key,
        &chain_hash,
        details,
        &mut nonces,
    ));
    let (result, _) = runner.execute(input);
    let tx = result.last_tx_receipt();
    assert!(tx.receipt.is_successful());
    let gas = get_gas_used(tx);
    blocks.push(snapshot(&runner, "withdraw", json!({ "tx": tx, "gas": gas, "gas_value": gas.value(result.last_batch_receipt().inner.gas_price) })));
    for (step, name) in ["escrow", "refund", "slash", "refund_after_slashing"]
        .into_iter()
        .enumerate()
    {
        let mut state = StateCheckpoint::<S>::new(
            runner.storage_manager().create_prover_storage(),
            &RT::default().kernel(),
        );
        let mut registry = SequencerRegistry::<S>::default();
        let bank = Bank::<S>::default();
        registry
            .remove_part_of_the_stake(
                &da_b,
                bank.id().clone().to_payable(),
                Amount::new(777),
                &mut state,
            )
            .unwrap();
        if step >= 1 {
            registry
                .add_to_stake(
                    bank.id().clone().to_payable(),
                    &da_b,
                    Amount::new(777),
                    &mut state,
                )
                .unwrap();
        }
        if step >= 2 {
            registry.slash_sequencer(&da_b, &mut state);
        }
        if step >= 3 {
            assert!(registry
                .add_to_stake(
                    bank.id().clone().to_payable(),
                    &da_b,
                    Amount::new(777),
                    &mut state
                )
                .is_err());
        }
        let balances = [1, 2, 3, 4]
            .map(|seed| TestRunner::<RT, S>::bank_gas_balance(&user(seed).address(), &mut state));
        let registrations = [0, 10, 11].map(|seed| {
            registry
                .is_sender_known(&MockAddress::new([seed; 32]), &mut state)
                .ok()
        });
        let (root, _, _, _, _) = state.materialize_update(*runner.state_root());
        blocks.push(json!({ "name": name, "root": root.to_string(), "receipt": null, "balances": balances, "registrations": registrations }));
    }
    json!({ "source_commit": "509a482949d553e3b178cac77290d9ecea15d9f2", "chain_hash": chain_hash, "blocks": blocks })
}

#[test]
fn historical_registry_execution_matches_dev() {
    let expected: Value = serde_json::from_str(BASELINE).unwrap();
    let chain_hash: [u8; 32] = serde_json::from_value(expected["chain_hash"].clone()).unwrap();
    let actual = replay(chain_hash);
    let actual_blocks = actual["blocks"].as_array().unwrap();
    let expected_blocks = expected["blocks"].as_array().unwrap();
    for (name, root) in PINNED_ROOTS {
        let block = expected_blocks
            .iter()
            .find(|block| block["name"] == name)
            .unwrap();
        assert_eq!(
            block["root"], root,
            "fixture root for {name} was regenerated"
        );
    }
    assert_eq!(actual_blocks.len(), expected_blocks.len());
    for (actual, expected) in actual_blocks.iter().zip(expected_blocks) {
        assert_eq!(
            actual, expected,
            "Historical replay diverged at {}",
            expected["name"]
        );
    }
}
