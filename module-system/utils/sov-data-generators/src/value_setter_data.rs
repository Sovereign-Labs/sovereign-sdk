use std::vec;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;

use super::*;

pub struct ValueSetterMessage {
    pub admin: Rc<DefaultPrivateKey>,
    pub messages: Vec<u32>,
}

pub struct ValueSetterMessages(Vec<ValueSetterMessage>);

impl ValueSetterMessages {
    pub fn new(messages: Vec<ValueSetterMessage>) -> Self {
        Self(messages)
    }
}

impl Default for ValueSetterMessages {
    /// This function will return a dummy value setter message containing one admin and two value setter messages.
    fn default() -> Self {
        Self::new(vec![ValueSetterMessage {
            admin: Rc::new(DefaultPrivateKey::generate()),
            messages: vec![99, 33],
        }])
    }
}

impl MessageGenerator for ValueSetterMessages {
    type Call = sov_value_setter::CallMessage;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)> {
        let mut messages = Vec::default();
        for value_setter_message in &self.0 {
            let admin = value_setter_message.admin.clone();
            let mut value_setter_admin_nonce = 0;

            for new_value in &value_setter_message.messages {
                let set_value_msg: sov_value_setter::CallMessage =
                    sov_value_setter::CallMessage::SetValue(*new_value);

                messages.push((admin.clone(), set_value_msg, value_setter_admin_nonce));

                value_setter_admin_nonce += 1;
            }
        }
        messages
    }

    fn create_tx(
        &self,
        sender: &DefaultPrivateKey,
        message: Self::Call,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Runtime::<DefaultContext>::encode_value_setter_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}
