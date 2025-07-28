//! Tests dispatching a message from one chain and receiving it on another

use sov_bank::Amount;
use sov_hyperlane_integration::test_recipient::Event;
use sov_hyperlane_integration::{
    CallMessage, Event as MailboxEvent, HyperlaneAddress, Message, MESSAGE_VERSION,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::{HexHash, HexString};
use sov_test_utils::{AsUser, TransactionTestCase};

use crate::runtime::{
    register_recipient, register_relayer_with_dummy_igp, setup, unlimited_gas_meter, Mailbox,
    TestRuntimeEvent, RT, S,
};

#[test]
fn test_send_receive_basic() {
    let (mut runner, admin, _user, relayer, ..) = setup();

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");

    let recipient_address: HexHash = [5u8; 32].into();
    let message_body = b"Hello, world!";
    let expected_message = Message {
        version: MESSAGE_VERSION,
        nonce: 0,
        origin_domain: config_value!("HYPERLANE_BRIDGE_DOMAIN"), // Signal that the sender is a Sovereign SDK chain, so the sender address can be parsed. This makes the test output nicer.
        sender: admin.address().to_sender(), // The sender doesn't matter for this test
        dest_domain: domain,
        recipient: recipient_address,
        body: message_body.to_vec().into(),
    };

    let domain = config_value!("HYPERLANE_BRIDGE_DOMAIN");
    register_relayer_with_dummy_igp(&mut runner, &relayer, domain);

    let message_id = expected_message.id(&mut unlimited_gas_meter()).unwrap();
    let admin_address = admin.address();
    register_recipient(&mut runner, &admin, recipient_address);

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Dispatch {
            domain,
            recipient: recipient_address,
            body: HexString(message_body.to_vec().try_into().unwrap()),
            metadata: None,
            relayer: Some(relayer.address()),
            gas_payment_limit: Amount::MAX,
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::Mailbox(MailboxEvent::DispatchId { id }) if *id == message_id
                )
            }));
        }),
    });

    runner.execute_transaction(TransactionTestCase {
        input: admin.create_plain_message::<RT, Mailbox<S>>(CallMessage::Process {
            message: HexString(expected_message.encode().0.try_into().unwrap()),
            metadata: HexString(vec![].try_into().unwrap()),
        }),
        assert: Box::new(move |result, _| {
            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::TestRecipient(Event::MessageReceivedGeneric { sender, body, .. })
                        if *sender == admin_address.to_sender() && body == &HexString::new(message_body.to_vec())
                )
            }),
            "Did not receive expected message. {:?}, tx receipt: {:?}",
            result.events,
            result.tx_receipt,
        );

            assert!(result.events.iter().any(|event| {
                matches!(
                    event,
                    TestRuntimeEvent::Mailbox(MailboxEvent::ProcessId {id }) if *id == message_id
                )
            }));
        }),
    });
}
