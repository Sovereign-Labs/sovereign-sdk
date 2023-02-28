// TODO remove it after all the macors are implemented.
#![allow(dead_code)]

use example_election::Election;
use example_value_setter::ValueSetter;
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    CallResponse, Context, DispatchCall, DispatchQuery, Error, Genesis, Module,
};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::{CacheLog, ValueReader};
use sovereign_db::state_db::StateDB;

/// dispatch_tx is a high level interface used by the sdk.
/// Transaction signature must be checked outside of this function.
fn dispatch_tx<C: Context, VR: ValueReader>(
    _tx_data: Vec<u8>,
    _context: C,
    _value_reader: VR,
) -> Result<(CallResponse, CacheLog), Error> {
    // 1. Create Storage (with fresh Cache)
    // 2. Deserialize tx
    // 3. deserialized_tx.dispatch(...)
    todo!()
}

/// Runtime defines modules registered in the rollup.
// #[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
struct Runtime<C: Context> {
    election: Election<C>,
    value_adder: ValueSetter<C>,
}

#[test]
fn test_demo() {
    type C = MockContext;
    let sender = MockPublicKey::try_from("admin").unwrap();
    let context = MockContext::new(sender);
    let temp_db = StateDB::temporary();
    type RT = Runtime<MockContext>;
    let storage = RT::genesis(temp_db).unwrap();

    // Call the election module.
    {
        let call_message = example_election::call::CallMessage::<C>::SetCandidates {
            names: vec!["candidate_1".to_owned()],
        };

        let serialized_message = RT::encode_election_call(call_message);
        let module = RT::decode_call(&serialized_message).unwrap();
        let result = module.dispatch_call(storage.clone(), &context);
        assert!(result.is_ok())
    }

    // Query the election module.
    {
        let query_message = example_election::query::QueryMessage::Result;

        let serialized_message = RT::encode_election_query(query_message);
        let module = RT::decode_query(&serialized_message).unwrap();

        let response = module.dispatch_query(storage);
        let _json_response = std::str::from_utf8(&response.response).unwrap();
    }
}
