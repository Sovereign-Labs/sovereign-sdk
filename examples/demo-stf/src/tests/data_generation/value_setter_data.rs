use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::default_signature::private_key::DefaultPrivateKey;

use super::*;

pub struct ValueSetterMessages {
    admin: Rc<DefaultPrivateKey>,
}

impl ValueSetterMessages {
    pub fn new(admin: DefaultPrivateKey) -> Self {
        Self {
            admin: Rc::new(admin),
        }
    }
}

impl MessageGenerator for ValueSetterMessages {
    type Call = sov_value_setter::call::CallMessage;

    fn create_messages(&self) -> Vec<(Rc<DefaultPrivateKey>, Self::Call, u64)> {
        let admin = self.admin.clone();
        let mut value_setter_admin_nonce = 0;
        let mut messages = Vec::default();

        let new_value = 99;

        let set_value_msg_1: sov_value_setter::call::CallMessage =
            sov_value_setter::call::CallMessage::SetValue(new_value);

        let new_value = 33;
        let set_value_msg_2 = sov_value_setter::call::CallMessage::SetValue(new_value);

        messages.push((admin.clone(), set_value_msg_1, value_setter_admin_nonce));

        value_setter_admin_nonce += 1;
        messages.push((admin, set_value_msg_2, value_setter_admin_nonce));

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
