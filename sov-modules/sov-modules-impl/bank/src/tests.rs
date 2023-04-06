use crate::{
    call,
    query::{self, QueryMessage},
    Bank,
};
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    Context, Module, ModuleInfo, PublicKey, Spec,
};
use sov_state::{ProverStorage, WorkingSet};

type C = MockContext;

#[test]
fn test_create_token() {
    let working_set = &mut WorkingSet::new(ProverStorage::temporary());
    let bank = &mut Bank::<C>::new();

    let sender = MockPublicKey::try_from("pub_key").unwrap();
    let sender_address = sender.to_address::<<C as Spec>::Address>();
    let sender_context = C::new(sender_address.clone());
    let minter_address = <C as Spec>::Address::try_from([0; 32].as_ref()).unwrap();

    let salt = 0;
    let token_name = "Token1".to_owned();

    let token_address = super::create_token_address::<C>(&token_name, &sender_address, salt);

    let create_token = call::CallMessage::CreateToken::<C> {
        salt: 0,
        token_name: "Token1".to_owned(),
        initial_balance: 100,
        minter_address: minter_address.clone(),
    };

    bank.call(create_token, &sender_context, working_set)
        .unwrap();

    let query = QueryMessage::GetBalance {
        user_address: minter_address,
        token_address,
    };

    let resp = bank.query(query, working_set);

    let query_response: query::BalanceResponse = serde_json::from_slice(&resp.response).unwrap();

    println!("RSP {:?}", query_response)
}
