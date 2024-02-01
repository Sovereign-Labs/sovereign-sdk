use std::vec;

use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;
use sov_modules_api::PrivateKey;
use sov_value_setter::ValueSetter;

use super::*;
use crate::EncodeCall;

const DEFAULT_CHAIN_ID: u64 = 0;
const DEFAULT_GAS_TIP: u64 = 0;
const DEFAULT_GAS_LIMIT: u64 = 0;

pub struct ValueSetterMessage<C: Context> {
    pub admin: Rc<C::PrivateKey>,
    pub messages: Vec<u32>,
}

pub struct ValueSetterMessages<C: Context> {
    pub messages: Vec<ValueSetterMessage<C>>,
}

impl<C: Context> ValueSetterMessages<C> {
    pub fn new(messages: Vec<ValueSetterMessage<C>>) -> Self {
        Self { messages }
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
    type Context = C;

    fn create_messages(&self) -> Vec<Message<Self::Context, Self::Module>> {
        let mut messages = Vec::default();
        for value_setter_message in &self.messages {
            let admin = value_setter_message.admin.clone();

            for (value_setter_admin_nonce, new_value) in
                value_setter_message.messages.iter().enumerate()
            {
                let set_value_msg: sov_value_setter::CallMessage =
                    sov_value_setter::CallMessage::SetValue(*new_value);

                messages.push(Message::new(
                    admin.clone(),
                    set_value_msg,
                    DEFAULT_CHAIN_ID,
                    DEFAULT_GAS_TIP,
                    DEFAULT_GAS_LIMIT,
                    value_setter_admin_nonce.try_into().unwrap(),
                ));
            }
        }
        messages
    }

    fn create_tx<Encoder: EncodeCall<Self::Module>>(
        &self,
        sender: &C::PrivateKey,
        message: <Self::Module as Module>::CallMessage,
        chain_id: u64,
        gas_tip: u64,
        gas_limit: u64,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<C> {
        let message = Encoder::encode_call(message);
        Transaction::<C>::new_signed_tx(sender, message, chain_id, gas_tip, gas_limit, nonce)
    }
}
