//! Tests receiving (aka "processing") messages

use sov_hyperlane_integration::test_recipient::{self, Event};
use sov_hyperlane_integration::{CallMessage, HyperlaneAddress, Ism, Message, MESSAGE_VERSION};
use sov_modules_api::macros::config_value;
use sov_modules_api::{HexHash, HexString, TxEffect};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::runtime::{
    register_recipient, register_recipient_with_ism, set_default_ism, setup, Mailbox,
    TestRuntimeEvent, RT, S,
};

#[test]
fn test_send_message_basic() {
    let (mut runner, admin, ..) = setup();

    let recipient_address: HexHash = [5u8; 32].into();
    let message_body = b"Hello, world!";
    let message = Message {
        version: MESSAGE_VERSION,
        nonce: 0,
        origin_domain: test_recipient::MAGIC_SOV_CHAIN_DOMAIN, // Signal that the sender is a Sovereign SDK chain, so the sender address can be parsed. This makes the test output nicer.
        sender: admin.address().to_sender(), // The sender doesn't matter for this test
        dest_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };

    let admin_address = admin.address();

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_reverted(),
                "No recipient or default ism is registered but the tx succeeded"
            );
        }),
    });

    register_recipient(&mut runner, &admin, recipient_address);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::TestRecipient(Event::MessageReceived { sender, body, .. }) if *sender == admin_address && body == &HexString::new(message_body.to_vec()).to_string()
                )
            }));
        }),
    });
}

/// Tests that messages are rejected if the destination domain is wrong (i.e. the message was intended for a different chain)
#[test]
fn test_send_message_to_wrong_domain() {
    let (mut runner, admin, ..) = setup();

    let recipient_address: HexHash = [5u8; 32].into();
    let message_body = b"Hello, world!";
    let domain: u32 = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    let message = Message {
        version: MESSAGE_VERSION,
        nonce: 0,
        origin_domain: test_recipient::MAGIC_SOV_CHAIN_DOMAIN, // Signal that the sender is a Sovereign SDK chain, so the sender address can be parsed.
        sender: admin.address().to_sender(), // The sender doesn't matter for this test
        dest_domain: domain.wrapping_add(1u32), // Modify the domain to be wrong
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };

    register_recipient(&mut runner, &admin, recipient_address);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(reverted) => {
                assert!(reverted
                    .reason
                    .to_string()
                    .contains("Invalid message destination domain"),
                "Unexpected revert reason. Expected: Invalid message destination domain. Actual: {}",
                reverted.reason
            );
            }
            _ => {
                panic!("Unexpected tx receipt: {:?}", result.tx_receipt);
            }
        }),
    });
}

/// Tests that message cannot be replayed
#[test]
fn test_replay_message_delivery() {
    let (mut runner, admin, ..) = setup();

    let recipient_address: HexHash = [5u8; 32].into();
    let message_body = b"Hello, world!";
    let message = Message {
        version: MESSAGE_VERSION,
        nonce: 0,
        origin_domain: test_recipient::MAGIC_SOV_CHAIN_DOMAIN, // Signal that the sender is a Sovereign SDK chain, so the sender address can be parsed.
        sender: admin.address().to_sender(), // The sender doesn't matter for this test
        dest_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };

    let admin_address = admin.address();
    register_recipient(&mut runner, &admin, recipient_address);
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::TestRecipient(Event::MessageReceived { sender, body, .. }) if *sender == admin_address && body == &HexString::new(message_body.to_vec()).to_string()
                )
            }));
        }),
    });
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(reverted) => {
                assert!(
                    reverted.reason.to_string().contains("already processed"),
                    "Unexpected revert reason. Expected: Message _ already processed. Actual: {}",
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
fn test_send_message_with_default_ism() {
    let (mut runner, admin, ..) = setup();

    let recipient_address: HexHash = [5u8; 32].into();
    let message_body = b"Hello, world!";
    let message = Message {
        version: MESSAGE_VERSION,
        nonce: 0,
        origin_domain: test_recipient::MAGIC_SOV_CHAIN_DOMAIN, // Signal that the sender is a Sovereign SDK chain, so the sender address can be parsed. This makes the test output nicer.
        sender: admin.address().to_sender(), // The sender doesn't matter for this test
        dest_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_reverted(),
                "No recipient or default ism is registered but the tx succeeded"
            );
        }),
    });

    // Set default ism
    set_default_ism(&mut runner, &admin, Ism::AlwaysTrust);
    // Now the message should be delivered
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::TestRecipient(Event::MessageReceivedUnknownRecipient { recipient, body, .. })
                        if *recipient == recipient_address && body == &HexString::new(message_body.to_vec())
                )
            }));
        }),
    });

    // Now register recipient with more strict ISM
    register_recipient_with_ism(
        &mut runner,
        &admin,
        recipient_address,
        Ism::TrustedRelayer {
            relayer: [11; 32].into(),
        },
    );

    // Since dedicated ISM takes precedence over default ism, this should fail as ISM condition
    // won't be met
    let message = Message {
        version: MESSAGE_VERSION,
        nonce: 1,
        origin_domain: test_recipient::MAGIC_SOV_CHAIN_DOMAIN, // Signal that the sender is a Sovereign SDK chain, so the sender address can be parsed. This makes the test output nicer.
        sender: admin.address().to_sender(), // The sender doesn't matter for this test
        dest_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(|result, _| {
            assert!(
                result.tx_receipt.is_reverted(),
                "Admin is not trusted relayer for recipient but tx succeeded"
            );
        }),
    });
}

/// Tests that messages are rejected by the "trusted relayer" ISM if the actual relayer is not the allowed relayer
#[test]
fn test_send_message_with_untrusted_relayer_to_trusted_relayer_ism() {
    let (mut runner, admin, test_user, ..) = setup();

    let recipient_address: HexHash = [5u8; 32].into();
    let message_body = b"Hello, world!";
    let message = Message {
        version: MESSAGE_VERSION,
        nonce: 0,
        origin_domain: test_recipient::MAGIC_SOV_CHAIN_DOMAIN, // Signal that the sender is a Sovereign SDK chain, so the sender address can be parsed.
        sender: admin.address().to_sender(), // The sender doesn't matter for this test
        dest_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };
    let test_user_address = test_user.address();
    let admin_address = admin.address();

    register_recipient_with_ism(
        &mut runner,
        &admin,
        recipient_address,
        Ism::TrustedRelayer {
            relayer: test_user.address().to_sender(),
        },
    );
    // Check that the message is rejected by the "trusted relayer" ISM
    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(reverted) => {
                assert!(
                    reverted.reason.to_string().contains(&format!(
                        "Only {} is trusted",
                        test_user_address.to_sender(),
                    )),
                    "Unexpected revert reason. Expected: Only {} is trusted. Actual: {}",
                    test_user_address.to_sender(),
                    reverted.reason
                );
            }
            _ => {
                panic!("Unexpected tx receipt: {:?}", result.tx_receipt);
            }
        }),
    });

    // Now try again with the correct relayer. The message should be accepted
    runner.execute_transaction(TransactionTestCase {
        input: test_user.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _|  {
            assert!(result.tx_receipt.is_successful(), "Message was not delivered successfully");
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::TestRecipient(Event::MessageReceived { sender, body, .. }) if *sender == admin_address && body == &HexString::new(message_body.to_vec()).to_string()
                )
            }));
        }),
    });
}

/// Tests that messages are rejected if the version is wrong
#[test]
fn test_send_message_with_wrong_version() {
    let (mut runner, admin, ..) = setup();

    let recipient_address: HexHash = [5u8; 32].into();
    let message_body = b"Hello, world!";
    let message = Message {
        version: 2, // Wrong version
        nonce: 0,
        origin_domain: test_recipient::MAGIC_SOV_CHAIN_DOMAIN, // Signal that the sender is a Sovereign SDK chain, so the sender address can be parsed.
        sender: admin.address().to_sender(), // The sender doesn't matter for this test
        dest_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"),
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };

    register_recipient(&mut runner, &admin, recipient_address);

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| match result.tx_receipt {
            TxEffect::Reverted(reverted) => {
                assert!(
                    reverted
                        .reason
                        .to_string()
                        .contains("Invalid message version"),
                    "Unexpected revert reason. Expected: Invalid message version. Actual: {}",
                    reverted.reason
                );
            }
            _ => {
                panic!("Unexpected tx receipt: {:?}", result.tx_receipt);
            }
        }),
    });
}
