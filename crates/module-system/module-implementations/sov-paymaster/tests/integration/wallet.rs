use sov_paymaster::CallMessage as PaymasterCallMessage;

use crate::utils::S;

#[test]
fn test_display_paymaster_call() {
    use std::str::FromStr;

    use sov_modules_api::sov_universal_wallet::schema::Schema;
    let msg = PaymasterCallMessage::<S>::SetPayerForSequencer {
        payer: FromStr::from_str("sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf")
            .unwrap(),
    };

    let schema = Schema::of_single_type::<sov_paymaster::CallMessage<S>>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        r#"SetPayerForSequencer { payer: sov1lzkjgdaz08su3yevqu6ceywufl35se9f33kztu5cu2spja5hyyf }"#
    );
}
