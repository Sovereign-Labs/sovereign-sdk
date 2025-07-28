use generators::{Message, MessageGenerator, Rc};
use sov_modules_api::{CryptoSpec, PrivateKey, Spec};
use sov_value_setter::ValueSetter;

use crate::*;

/// Defines a message that can be sent to the value setter module.
pub struct ValueSetterMessage<S: Spec> {
    /// The admin of the value setter.
    pub admin: Rc<<S::CryptoSpec as CryptoSpec>::PrivateKey>,
    /// The messages to be sent to the value setter. This is a list of values to be set.
    pub messages: Vec<u32>,
}

/// Defines a set of messages that can be sent to the value setter module.
pub struct ValueSetterMessages<S: Spec> {
    /// The messages to be sent to the value setter.
    pub messages: Vec<ValueSetterMessage<S>>,
}

impl<S: Spec> ValueSetterMessages<S> {
    /// Creates a new [`ValueSetterMessages`] from a list of [`ValueSetterMessage`]s.
    pub fn new(messages: Vec<ValueSetterMessage<S>>) -> Self {
        Self { messages }
    }

    /// Returns a message containing one admin and two value setter messages.
    pub fn prepopulated() -> Self {
        Self::new(vec![ValueSetterMessage {
            admin: Rc::new(<<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate()),
            messages: vec![99, 33],
        }])
    }
}

impl<S: Spec> MessageGenerator for ValueSetterMessages<S> {
    type Module = ValueSetter<S>;
    type Spec = S;

    fn create_messages(
        &self,
        chain_id: u64,
        max_priority_fee_bips: PriorityFeeBips,
        max_fee: Amount,
        gas_usage: Option<<Self::Spec as Spec>::Gas>,
    ) -> Vec<Message<Self::Spec, Self::Module>> {
        let mut messages = Vec::default();
        for value_setter_message in &self.messages {
            let admin = value_setter_message.admin.clone();

            for (value_setter_admin_nonce, new_value) in
                value_setter_message.messages.iter().enumerate()
            {
                let set_value_msg: sov_value_setter::CallMessage<S> =
                    sov_value_setter::CallMessage::SetValue {
                        value: *new_value,
                        gas: None,
                    };

                messages.push(Message::new(
                    admin.clone(),
                    set_value_msg,
                    chain_id,
                    max_priority_fee_bips,
                    max_fee,
                    gas_usage.clone(),
                    value_setter_admin_nonce.try_into().unwrap(),
                ));
            }
        }
        messages
    }
}
