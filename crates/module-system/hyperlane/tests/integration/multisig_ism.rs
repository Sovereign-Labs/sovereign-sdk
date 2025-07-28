//! Test for the multisig ISM.

use std::str::FromStr;

use sov_bank::Amount;
use sov_hyperlane_integration::crypto::compute_hash_for_signatures;
use sov_hyperlane_integration::test_recipient::Event;
use sov_hyperlane_integration::{CallMessage, Ism, Message};
use sov_modules_api::macros::config_value;
use sov_modules_api::{Address, BasicGasMeter, Context, GasPrice, GasUnit, HexString, TxEffect};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::runtime::{
    random_validator, register_recipient_with_ism, setup, sign, unlimited_gas_meter, Mailbox,
    TestRuntimeEvent, RT, S,
};

pub struct MultisigIsmTestData {
    pub message: Message,
    pub validators: Vec<HexString<[u8; 20]>>,
    pub signatures: Vec<Vec<u8>>,
    pub metadata_without_signatures: HexString,
}

const ORIGIN_DOMAIN: u32 = 1234u32;

fn get_message() -> Message {
    Message {
        version: 3,
        nonce: 69,
        origin_domain: ORIGIN_DOMAIN,
        sender: HexString::from_str(
            "0xafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafaf",
        )
        .unwrap(),
        dest_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient: HexString::from_str(
            "0xbebebebebebebebebebebebebebebebebebebebebebebebebebebebebebebebe",
        )
        .unwrap(),
        body: HexString(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
    }
}

pub fn get_multisig_ism_test_data() -> MultisigIsmTestData {
    let gas_meter = &mut unlimited_gas_meter();
    let message = get_message();

    // assume some arbitrary state of the counterparty
    let origin_merkle_tree = HexString([12; 32]);
    let checkpoint_root = HexString([21; 32]);
    // different than nonce to make sure it's parsed from metadata
    let checkpoint_index = message.nonce + 1;

    let digest = compute_hash_for_signatures(
        &message,
        &origin_merkle_tree,
        &checkpoint_root,
        checkpoint_index,
        gas_meter,
    )
    .unwrap();

    let (sk_0, validator_0) = random_validator();
    let signature_0 = sign(digest.0, &sk_0).0.to_vec();

    let (sk_1, validator_1) = random_validator();
    let signature_1 = sign(digest.0, &sk_1).0.to_vec();

    let (sk_2, validator_2) = random_validator();
    let signature_2 = sign(digest.0, &sk_2).0.to_vec();

    let mut metadata_without_signatures = vec![];
    metadata_without_signatures.extend_from_slice(&origin_merkle_tree.0);
    metadata_without_signatures.extend_from_slice(&checkpoint_root.0);
    metadata_without_signatures.extend_from_slice(&checkpoint_index.to_be_bytes());

    MultisigIsmTestData {
        message,
        validators: vec![validator_0, validator_1, validator_2],
        signatures: vec![signature_0, signature_1, signature_2],
        metadata_without_signatures: metadata_without_signatures.into(),
    }
}

fn dummy_context() -> Context<S> {
    Context::new(
        Address::new([0; Address::LENGTH]),
        Default::default(),
        Address::new([0; Address::LENGTH]),
        [1; 32].into(),
    )
}

#[test]
fn test_verify_valid() {
    let data = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    // Put two valid signatures in the metadata
    metadata.0.extend(data.signatures[0].iter());
    metadata.0.extend(data.signatures[1].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };
    assert!(ism
        .verify(
            &dummy_context(),
            &data.message,
            &metadata,
            &mut unlimited_gas_meter()
        )
        .is_ok());
}

#[test]
fn test_verify_wrong_message() {
    let mut data = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    // Put two valid signatures in the metadata
    metadata.0.extend(data.signatures[0].iter());
    metadata.0.extend(data.signatures[1].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };
    // Tweak the message so that the signatures no longer fit it.
    data.message.nonce = u32::MAX;
    assert!(ism
        .verify(
            &dummy_context(),
            &data.message,
            &metadata,
            &mut unlimited_gas_meter()
        )
        .is_err());
}

#[test]
fn test_verify_duplicate_signatures() {
    let data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    // Put the same signature twice
    metadata.0.extend(data.signatures[0].iter());
    metadata.0.extend(data.signatures[0].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };
    assert!(ism
        .verify(
            &dummy_context(),
            &data.message,
            &metadata,
            &mut unlimited_gas_meter()
        )
        .is_err());
}

#[test]
fn test_verify_signature_check_overflow() {
    let data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    metadata.0.extend(data.signatures[0].iter());

    let ism: Ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: u32::MAX,
    };
    let v = ism.verify(
        &dummy_context(),
        &data.message,
        &metadata,
        &mut unlimited_gas_meter(),
    );

    assert!(v.is_err());
}

#[test]
fn test_verify_not_enough_signatures() {
    let data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    // Only put one signature in the metadata
    metadata.0.extend(data.signatures[0].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };
    assert!(ism
        .verify(
            &dummy_context(),
            &data.message,
            &metadata,
            &mut unlimited_gas_meter()
        )
        .is_err());
}

#[test]
fn test_verify_invalid_signatures() {
    let mut data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    // Modify the first signature so that it's invalid
    data.signatures[0][0] = data.signatures[0][0].wrapping_add(1);
    metadata.0.extend(data.signatures[0].iter());
    metadata.0.extend(data.signatures[1].iter());
    metadata.0[64] = 27;
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };
    assert!(ism
        .verify(
            &dummy_context(),
            &data.message,
            &metadata,
            &mut unlimited_gas_meter()
        )
        .is_err());
}

#[test]
fn test_verify_too_many_signatures() {
    let data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    // Put three signatures in the metadata
    metadata.0.extend(data.signatures[0].iter());
    metadata.0.extend(data.signatures[1].iter());
    metadata.0.extend(data.signatures[2].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };
    assert!(ism
        .verify(
            &dummy_context(),
            &data.message,
            &metadata,
            &mut unlimited_gas_meter()
        )
        .is_err());
}

#[test]
fn test_verify_out_of_order_signatures() {
    let data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;

    // Put the last two signatures in the metadata
    metadata.0.extend(data.signatures[2].iter());
    metadata.0.extend(data.signatures[1].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };
    assert!(ism
        .verify(
            &dummy_context(),
            &data.message,
            &metadata,
            &mut unlimited_gas_meter()
        )
        .is_ok());
}

#[test]
fn test_verify_not_enough_gas() {
    let data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;

    // Put the last two signatures in the metadata
    metadata.0.extend(data.signatures[2].iter());
    metadata.0.extend(data.signatures[1].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };
    let mut gas_meter = BasicGasMeter::new_with_funds_and_gas(
        Amount(100),
        GasUnit::from([1, 1]),
        GasPrice::from([Amount(100), Amount(100)]),
    );
    assert!(ism
        .verify(&dummy_context(), &data.message, &metadata, &mut gas_meter)
        .is_err());
}

/// Run an end-to-end test that verifies a valid message is processed correctly with the multisig ISM
#[test]
fn test_verify_valid_end_to_end() {
    let data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    // Put two valid signatures in the metadata
    metadata.0.extend(data.signatures[0].iter());
    metadata.0.extend(data.signatures[1].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };

    let (mut runner, admin, ..) = setup();

    register_recipient_with_ism(
        &mut runner,
        &admin,
        HexString::from_str("0xbebebebebebebebebebebebebebebebebebebebebebebebebebebebebebebebe")
            .unwrap(),
        ism,
    );

    let expected_sender =
        HexString::from_str("0xafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafafaf")
            .unwrap();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(get_message().encode().0.try_into().unwrap()),
            metadata: HexString(metadata.0.try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::TestRecipient(Event::MessageReceivedGeneric { sender, body, .. })
                        if *sender == expected_sender && body == &HexString::new(vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]),
                )
            }),
            "Did not receive expected message. {:?}, tx receipt: {:?}",
            result.events,
            result.tx_receipt,
        );
        }),
    });
}

/// Run an end-to-end test that verifies an invalid message is rejected with the multisig ISM
#[test]
fn test_verify_invalid_end_to_end() {
    let data: MultisigIsmTestData = get_multisig_ism_test_data();
    let mut metadata = data.metadata_without_signatures;
    // Duplicate the first signature
    metadata.0.extend(data.signatures[0].iter());
    metadata.0.extend(data.signatures[0].iter());
    let ism = Ism::MessageIdMultisig {
        validators: data.validators.try_into().unwrap(),
        threshold: 2,
    };

    let (mut runner, admin, ..) = setup();

    register_recipient_with_ism(
        &mut runner,
        &admin,
        HexString::from_str("0xbebebebebebebebebebebebebebebebebebebebebebebebebebebebebebebebe")
            .unwrap(),
        ism,
    );

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(get_message().encode().0.try_into().unwrap()),
            metadata: HexString(metadata.0.try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(reverted) => {
                assert!(reverted.reason.to_string().contains("Not enough unique validators signed"), "Unexpected revert reason. Expected: Not enough unique validators signed the message. Actual: {}", reverted.reason);
            }
            _ => {
                panic!("Unexpected tx receipt: {:?}", result.tx_receipt);
            }
        }),
    });
}
