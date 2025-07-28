use std::str::FromStr;

use sov_bank::{CallMessage, Coins, TokenId};
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::{Amount, SafeVec, Spec};
use sov_test_utils::TestSpec;

type S = TestSpec;

#[test]
fn test_create_token() {
    let schema = Schema::of_single_type::<CallMessage<S>>().unwrap();
    let msg = CallMessage::CreateToken::<S> {
        token_name: "my-token".try_into().unwrap(),
        token_decimals: Some(6),
        initial_balance: Amount::new(100_000_000),
        supply_cap: None,
        mint_to_address: <S as Spec>::Address::from_str(
            "sov1x3jtvq0zwhj2ucsc4hqugskvralrulxvf53vwtkred93s85ar2a",
        )
        .unwrap(),
        admins: SafeVec::new(),
    };

    assert_eq!(schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(), "CreateToken { token_name: \"my-token\", token_decimals: 6, initial_balance: 100000000, mint_to_address: sov1x3jtvq0zwhj2ucsc4hqugskvralrulxvf53vwtkred93s85ar2a, admins: [], supply_cap: None }");
}

#[test]
fn test_transfer() {
    let schema = Schema::of_single_type::<CallMessage<S>>().unwrap();
    let msg: CallMessage<S> = CallMessage::Transfer {
        to: <S as Spec>::Address::from_str(
            "sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv",
        )
        .unwrap(),
        coins: Coins {
            amount: Amount::new(100_000),
            token_id: TokenId::from_str(
                "token_1zut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzurq2akgf6",
            )
            .unwrap(),
        },
    };

    assert_eq!(schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(), "Transfer to address sov1pv9skzctpv9skzctpv9skzctpv9skzctpv9skzctpv9skqm7ehv 0.1 coins of token ID token_1zut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzut3w9chzurq2akgf6.");
}
