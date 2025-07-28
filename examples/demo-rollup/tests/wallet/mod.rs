use std::str::FromStr;

use demo_stf::runtime::{Runtime, RuntimeCall};
use sov_bank::{CallMessage, Coins, TokenId};
use sov_modules_api::sov_universal_wallet::schema::{ChainData, RollupRoots, Schema};
use sov_modules_api::transaction::{Transaction, UnsignedTransaction, VersionedTx};
use sov_modules_api::{Address, Amount, DispatchCall, PrivateKey, Spec};
use sov_modules_macros::config_value;
use sov_test_utils::{
    TestUser, TEST_DEFAULT_GAS_LIMIT, TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE,
};

use crate::test_helpers::{DemoRollupSpec, CHAIN_HASH};

type S = DemoRollupSpec;

fn make_unsigned_tx() -> UnsignedTransaction<Runtime<S>, S> {
    let msg: RuntimeCall<S> = RuntimeCall::Bank(CallMessage::Mint {
        mint_to_address: <S as Spec>::Address::from_str(
            "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv",
        )
        .unwrap(),
        coins: Coins {
            amount: Amount::new(10_000),
            token_id: TokenId::from_str(
                "token_1zut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzurq2akgf6",
            )
            .unwrap(),
        },
    });
    UnsignedTransaction::<_, S>::new(
        msg,
        config_value!("CHAIN_ID"),
        TEST_DEFAULT_MAX_PRIORITY_FEE,
        TEST_DEFAULT_MAX_FEE,
        0,
        Some(TEST_DEFAULT_GAS_LIMIT.into()),
    )
}

#[test]
fn test_transfer_template() {
    let expected_call = RuntimeCall::Bank(CallMessage::Transfer {
        to: <S as Spec>::Address::from_str(
            "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv",
        )
        .unwrap(),
        coins: Coins {
            amount: Amount::new(4342),
            token_id: TokenId::from_str(
                "token_1zut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzurq2akgf6",
            )
            .unwrap(),
        },
    });
    let expected_bytes = <Runtime<S> as DispatchCall>::encode(&expected_call);

    let template_input = r#"{
        "to": { "Standard": [11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11, 11] },
        "amount": 4342,
        "token_id": [23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 23, 6]
    }"#;
    let schema = Schema::of_rollup_types_with_chain_data::<
        Transaction<Runtime<S>, S>,
        UnsignedTransaction<Runtime<S>, S>,
        RuntimeCall<S>,
        Address,
    >(ChainData {
        chain_id: 4321,
        chain_name: "TestChain".to_string(),
    })
    .unwrap();
    let template_transaction = schema
        .fill_template_from_json(
            RollupRoots::RuntimeCall as usize,
            "transfer",
            template_input,
        )
        .unwrap();
    assert_eq!(template_transaction, expected_bytes);
}

#[test]
fn test_display_unsigned_tx() {
    let unsigned_tx = make_unsigned_tx();
    let unsigned_data = borsh::to_vec(&unsigned_tx).unwrap();
    let schema = Schema::of_rollup_types_with_chain_data::<
        Transaction<Runtime<S>, S>,
        UnsignedTransaction<Runtime<S>, S>,
        RuntimeCall<S>,
        Address,
    >(ChainData {
        chain_id: 4321,
        chain_name: "TestChain".to_string(),
    })
    .unwrap();
    assert_eq!(
        schema
            .display(
                schema
                    .rollup_expected_index(RollupRoots::UnsignedTransaction)
                    .unwrap(),
                &unsigned_data
            )
            .unwrap(),
        r#"{ runtime_call: Bank.Mint { coins: 0.01 coins of token ID token_1zut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzurq2akgf6, mint_to_address: sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv }, generation: 0, details: { max_priority_fee_bips: 0, max_fee: 100000000000, gas_limit: [1000000000, 1000000000], chain_id: 4321 } }"#
    );
}

#[test]
fn test_display_signed_tx() {
    let unsigned_tx = make_unsigned_tx();
    let signer = TestUser::<S>::generate(Amount::ZERO);
    let signed_tx = Transaction::new_signed_tx(signer.private_key(), &CHAIN_HASH, unsigned_tx);
    let signed_data = borsh::to_vec(&signed_tx).unwrap();
    let schema = Schema::of_rollup_types_with_chain_data::<
        Transaction<Runtime<S>, S>,
        UnsignedTransaction<Runtime<S>, S>,
        RuntimeCall<S>,
        Address,
    >(ChainData {
        chain_id: 4321,
        chain_name: "TestChain".to_string(),
    })
    .unwrap();

    let signature_display = match signed_tx.versioned_tx {
        VersionedTx::V0(inner) => hex::encode(borsh::to_vec(&inner.signature).unwrap()),
    };

    let pubkey_display = hex::encode(borsh::to_vec(&signer.private_key.pub_key()).unwrap());

    assert_eq!(
        schema
            .display(
                schema
                    .rollup_expected_index(RollupRoots::Transaction)
                    .unwrap(),
                &signed_data
            )
            .unwrap(),
        format!("{{ versioned_tx: V0 {{ signature: {{ msg_sig: 0x{signature_display} }}, pub_key: {{ pub_key: 0x{pubkey_display} }}, runtime_call: Bank.Mint {{ coins: 0.01 coins of token ID token_1zut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzurq2akgf6, mint_to_address: sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv }}, generation: 0, details: {{ max_priority_fee_bips: 0, max_fee: 100000000000, gas_limit: [1000000000, 1000000000], chain_id: 4321 }} }} }}")
    );
}

#[ignore = "Ignored for rapid schema iteration, re-enable when CI timebomb goes off"]
#[test]
fn detect_schema_has_breaking_change() {
    let current_hash = [0u8; 32];
    assert_eq!(CHAIN_HASH, current_hash, "The chain hash changed. Update the \"current_hash\" value in this test but be aware: this is a breaking change for any production rollups.");
}
