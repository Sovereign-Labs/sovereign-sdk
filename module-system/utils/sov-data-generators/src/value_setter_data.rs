use std::marker::PhantomData;
use std::vec;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_value_setter::ValueSetter;

use super::*;
use crate::EncodeCall;

pub struct ValueSetterMessage {
    pub admin: Rc<DefaultPrivateKey>,
    pub messages: Vec<u32>,
}

pub struct ValueSetterMessages<C> {
    pub messages: Vec<ValueSetterMessage>,
    phantom_context: PhantomData<C>,
}

impl<C: Context> ValueSetterMessages<C> {
    pub fn new(messages: Vec<ValueSetterMessage>) -> Self {
        Self {
            messages,
            phantom_context: PhantomData::default(),
        }
    }
}

impl Default for ValueSetterMessages<DefaultContext> {
    /// This function will return a dummy value setter message containing one admin and two value setter messages.
    fn default() -> Self {
        Self::new(vec![ValueSetterMessage {
            admin: Rc::new(DefaultPrivateKey::generate()),
            messages: vec![99, 33],
        }])
    }
}

impl<C: Context> MessageGenerator for ValueSetterMessages<C> {
    type Module = ValueSetter<C>;

    fn create_messages(
        &self,
    ) -> Vec<(
        Rc<DefaultPrivateKey>,
        <Self::Module as Module>::CallMessage,
        u64,
    )> {
        let mut messages = Vec::default();
        for value_setter_message in &self.messages {
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

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &DefaultPrivateKey,
        message: <Self::Module as Module>::CallMessage,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<DefaultContext> {
        let message = Encoder::encode_call(message);
        Transaction::<DefaultContext>::new_signed_tx(sender, message, nonce)
    }
}
