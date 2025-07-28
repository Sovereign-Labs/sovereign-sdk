//! Tests for validator announcements

use secp256k1::SecretKey;
use sov_hyperlane_integration::crypto::{AnnouncementHash, DomainHash, EthSignHash, HashKind};
use sov_hyperlane_integration::test_recipient::Event as TestRecipientEvent;
use sov_hyperlane_integration::{
    CallMessage, EthAddress, Event as MailboxEvent, StorageLocation, ValidatorSignature,
    MAILBOX_ADDR,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::{HexHash, TxEffect};
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::{AsUser, TestUser, TransactionTestCase};

use crate::runtime::{
    random_validator, register_recipient, setup, sign, unlimited_gas_meter, Mailbox, TestRuntime,
    TestRuntimeEvent, RT, S,
};

#[test]
fn test_correct_validator_announcement() {
    let (mut runner, admin, ..) = setup();

    let recipient_address: HexHash = [5u8; 32].into();
    register_recipient(&mut runner, &admin, recipient_address);

    let (val_sk, val_addr) = random_validator();
    let location = "file:///dev/null".parse().unwrap();
    let signature = create_signature(
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        &MAILBOX_ADDR,
        HashKind::HyperlaneAnnouncement,
        &location,
        &val_sk,
    );

    // Announce first signatures location
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Announce {
            validator_address: val_addr,
            storage_location: location.clone(),
            signature,
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::Mailbox(MailboxEvent::ValidatorAnnouncement { address, storage_location })
                        if *address == val_addr && storage_location == &location
                )
            }), "Did not receive expected event of validator announcement. {:?}", result.events);
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::TestRecipient(TestRecipientEvent::AnnouncementReceived { address, storage_location })
                        if *address == val_addr && storage_location == &location
                )
            }), "Did not receive expected event of recipient getting announcement. {:?}", result.events);
        }),
    });

    let other_location = "file:///dev/random".parse().unwrap();
    let signature = create_signature(
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        &MAILBOX_ADDR,
        HashKind::HyperlaneAnnouncement,
        &other_location,
        &val_sk,
    );

    // Announce second signatures location
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Announce {
            validator_address: val_addr,
            storage_location: other_location.clone(),
            signature,
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::Mailbox(MailboxEvent::ValidatorAnnouncement { address, storage_location })
                        if *address == val_addr && storage_location == &other_location
                )
            }), "Did not receive expected event of validator announcement. {:?}", result.events);
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::TestRecipient(TestRecipientEvent::AnnouncementReceived { address, storage_location })
                        if *address == val_addr && storage_location == &other_location
                )
            }), "Did not receive expected event of recipient getting announcement. {:?}", result.events);
        }),
    });
}

fn assert_invalid_validator_announcement(
    runner: &mut TestRunner<TestRuntime<S>, S>,
    admin: &TestUser<S>,
    validator_address: EthAddress,
    storage_location: StorageLocation,
    signature: ValidatorSignature,
    expected_err: String,
) {
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Announce {
            validator_address,
            storage_location,
            signature,
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(reverted) => {
                assert!(
                    reverted.reason.to_string().contains(&expected_err),
                    "Unexpected revert reason. Expected to contain: {}. Actual: {}",
                    expected_err,
                    reverted.reason
                );
            }
            _ => {
                panic!("Unexpected tx receipt: {:?}", result.tx_receipt);
            }
        }),
    });
}

#[test]
fn test_invalid_signature() {
    let (mut runner, admin, ..) = setup();

    let (val_sk, val_addr) = random_validator();
    let location = "file:///dev/null".parse().unwrap();

    let invalid_domain = create_signature(
        1234,
        &MAILBOX_ADDR,
        HashKind::HyperlaneAnnouncement,
        &location,
        &val_sk,
    );

    let invalid_mailbox = create_signature(
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        &[1; 32],
        HashKind::HyperlaneAnnouncement,
        &location,
        &val_sk,
    );

    let invalid_hash_kind = create_signature(
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        &MAILBOX_ADDR,
        HashKind::Hyperlane,
        &location,
        &val_sk,
    );

    let invalid_location = create_signature(
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        &MAILBOX_ADDR,
        HashKind::HyperlaneAnnouncement,
        &"foo".parse().unwrap(),
        &val_sk,
    );

    let (other_sk, _) = random_validator();
    let invalid_signer = create_signature(
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        &MAILBOX_ADDR,
        HashKind::HyperlaneAnnouncement,
        &location,
        &other_sk,
    );

    for sig in [
        invalid_domain,
        invalid_mailbox,
        invalid_hash_kind,
        invalid_location,
        invalid_signer,
    ] {
        assert_invalid_validator_announcement(
            &mut runner,
            &admin,
            val_addr,
            location.clone(),
            sig,
            "Recovered address doesn't match announced address".into(),
        );
    }

    let good_sig = create_signature(
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        &MAILBOX_ADDR,
        HashKind::HyperlaneAnnouncement,
        &location,
        &val_sk,
    );

    // invalid address
    let (_, other_addr) = random_validator();
    assert_invalid_validator_announcement(
        &mut runner,
        &admin,
        other_addr,
        location.clone(),
        good_sig,
        "Recovered address doesn't match announced address".into(),
    );

    // invalid location
    assert_invalid_validator_announcement(
        &mut runner,
        &admin,
        val_addr,
        "foo".parse().unwrap(),
        good_sig,
        "Recovered address doesn't match announced address".into(),
    );
}

#[test]
fn test_duplicate_announcement() {
    let (mut runner, admin, ..) = setup();

    let (val_sk, val_addr) = random_validator();
    let location = "file:///dev/null".parse().unwrap();
    let signature = create_signature(
        config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        &MAILBOX_ADDR,
        HashKind::HyperlaneAnnouncement,
        &location,
        &val_sk,
    );

    let location_clone = location.clone();
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Announce {
            validator_address: val_addr,
            storage_location: location.clone(),
            signature,
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::Mailbox(MailboxEvent::ValidatorAnnouncement { address, storage_location })
                        if *address == val_addr && storage_location == &location_clone
                )
            }), "Did not receive expected event of validator announcement. {:?}", result.events);
        }),
    });

    // second announcement of the same location
    assert_invalid_validator_announcement(
        &mut runner,
        &admin,
        val_addr,
        location.clone(),
        signature,
        "already announced location".into(),
    );
}

pub fn create_signature(
    domain: u32,
    mailbox_addr: &[u8; 32],
    kind: HashKind,
    location: &StorageLocation,
    sk: &SecretKey,
) -> ValidatorSignature {
    let gas_meter = &mut unlimited_gas_meter();

    let domain_hash = DomainHash::new(domain, mailbox_addr, kind, gas_meter).unwrap();
    let announcement_hash = AnnouncementHash::new(domain_hash, location, gas_meter).unwrap();
    let digest = EthSignHash::new(announcement_hash.0, gas_meter).unwrap();

    sign(digest.0, sk)
}
