use super::*;

fn value_setter_call_messages() -> Vec<(MockPublicKey, value_setter::call::CallMessage, u64)> {
    let value_setter_admin = MockPublicKey::from("value_setter_admin");
    let mut value_setter_admin_nonce = 0;
    let mut messages = Vec::default();

    let new_value = 99;

    let set_value_msg_1 =
        value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue { new_value });

    let new_value = 33;
    let set_value_msg_2 =
        value_setter::call::CallMessage::DoSetValue(value_setter::call::SetValue { new_value });

    messages.push((
        value_setter_admin.clone(),
        set_value_msg_1,
        value_setter_admin_nonce,
    ));

    value_setter_admin_nonce += 1;
    messages.push((
        value_setter_admin,
        set_value_msg_2,
        value_setter_admin_nonce,
    ));

    messages
}

pub struct ValueSetterMessages {}

impl MessageGenerator for ValueSetterMessages {
    type Call = value_setter::call::CallMessage;

    fn create_messages(&self) -> Vec<(MockPublicKey, Self::Call, u64)> {
        value_setter_call_messages()
    }

    fn create_tx(
        &self,
        sender: MockPublicKey,
        message: Self::Call,
        nonce: u64,
        _is_last: bool,
    ) -> Transaction<MockContext> {
        Transaction::<MockContext>::new(
            Runtime::<MockContext>::encode_value_setter_call(message),
            sender,
            MockSignature::default(),
            nonce,
        )
    }
}
