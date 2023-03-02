//! Basic Runtime Example
//! This `Runtime` serves as a basic example of how to wire up module system and trigger the rollup logic.

#![allow(dead_code)]

use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    CallResponse, Context, DispatchCall, DispatchQuery, Error, Genesis, Module,
};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::{CacheLog, JmtStorage, ValueReader};
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

/// On a high level, the rollup node receives serialized call messages from the DA layer and executes them as atomic transactions.
/// Upon reception, the message has to be deserialized and forwarded to an appropriate module.
///
/// The module specific logic is implemented by module creators, but all the glue code responsible for message
/// deserialization/forwarding is handled by a rollup `runtime`.
///
/// In order to define the runtime we need to specify all the modules supported by our rollup (see the `Runtime` struct bellow)
///
/// The `Runtime` together with associated interfaces (`Genesis`, `DispatchCall`, `DispatchQuery`, `MessageCodec`)
/// and derive macros defines:
/// - how the rollup modules are wired up together.
/// - how the state of the rollup is initialized.
/// - how messages are dispatched to appropriate modules.
///
/// Runtime lifecycle:
///
/// 1. Initialization:
///     When a rollup is deployed for the first time, it needs to set its genesis state.
///     The `#[derive(Genesis)` macro will generate `Runtime::genesis(config)` method which returns
///     `Storage` with the initialized state.
///
/// 2. Calls:      
///     The `Module` interface defines a `call` method which accepts a module-defined type and triggers the specific `module logic.`
///     In general, the point of a call is to change the module state, but if the call throws an error,
///     no state is updated (the transaction is reverted).
///
/// 3. Queries:
///    The `Module` interface defines a `query` method, which allows querying the state of the module.
///     Queries are read only i.e they don't change the state of the rollup.
///     
/// `#[derive(MessageCodec)` adds deserialization capabilities to the `Runtime` (implements `decode_call` method).
/// `Runtime::decode_call` accepts serialized call message and returns a type that implements the `DispatchCall` trait.
///  The `DispatchCall` implementation (derived by a macro) forwards the message to the appropriate module and executes its `call` method.
///
/// Similar mechanism works for queries with the difference that queries are submitted by users directly to the rollup node
/// instead of going through the DA layer.
#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
pub struct Runtime<C: Context> {
    /// Definition of the first module in the rollup (must implement the sov_modules_api::Module trait).
    election: example_election::Election<C>,
    /// Definition of the second module in the rollup (must implement the sov_modules_api::Module trait).
    value_setter: example_value_setter::ValueSetter<C>,
}

fn run_example() {
    type C = MockContext;
    let db = StateDB::temporary();
    let sender = MockPublicKey::try_from("admin").unwrap();

    type RT = Runtime<C>;

    // Initialize the rollup: Call genesis on the Runtime
    let storage = RT::genesis(db).unwrap();

    let admin_context = C::new(sender, storage.clone());

    // Election module
    // Send candidates
    {
        let set_candidates_message = example_election::call::CallMessage::<C>::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        let serialized_message = RT::encode_election_call(set_candidates_message);
        let module = RT::decode_call(&serialized_message).unwrap();
        let result = module.dispatch_call(storage.clone(), &admin_context);
        assert!(result.is_ok())
    }

    let voters = vec![
        MockPublicKey::try_from("voter_1").unwrap(),
        MockPublicKey::try_from("voter_2").unwrap(),
        MockPublicKey::try_from("voter_3").unwrap(),
    ];

    // Add voters
    {
        for voter in voters.iter() {
            let add_voter_message =
                example_election::call::CallMessage::<C>::AddVoter(voter.clone());

            let serialized_message = RT::encode_election_call(add_voter_message);
            let module = RT::decode_call(&serialized_message).unwrap();

            let result = module.dispatch_call(storage.clone(), &admin_context);
            assert!(result.is_ok())
        }
    }

    // Vote
    {
        for voter in voters {
            let voter_context = C::new(voter, storage.clone());
            let vote_message = example_election::call::CallMessage::<C>::Vote(1);

            let serialized_message = RT::encode_election_call(vote_message);
            let module = RT::decode_call(&serialized_message).unwrap();

            let result = module.dispatch_call(storage.clone(), &voter_context);
            assert!(result.is_ok())
        }
    }

    // Freeze
    {
        let freeze_message = example_election::call::CallMessage::<C>::FreezeElection;

        let serialized_message = RT::encode_election_call(freeze_message);
        let module = RT::decode_call(&serialized_message).unwrap();

        let result = module.dispatch_call(storage.clone(), &admin_context);
        assert!(result.is_ok())
    }

    // Query the election module.
    {
        let query_message = example_election::query::QueryMessage::Result;

        let serialized_message = RT::encode_election_query(query_message);
        let module = RT::decode_query(&serialized_message).unwrap();

        let query_response = module.dispatch_query(storage.clone());

        let response: example_election::query::Response =
            serde_json::from_slice(&query_response.response).unwrap();

        assert_eq!(
            response,
            example_election::query::Response::Result(Some(example_election::Candidate {
                name: "candidate_2".to_owned(),
                count: 3
            }))
        )
    }

    // ValueSetter module
    // Set new value
    let new_value = 99;
    {
        let set_value_msg = example_value_setter::call::CallMessage::DoSetValue(
            example_value_setter::call::SetValue { new_value },
        );

        let serialized_message = RT::encode_value_setter_call(set_value_msg);
        let module = RT::decode_call(&serialized_message).unwrap();
        let result = module.dispatch_call(storage.clone(), &admin_context);

        assert!(result.is_ok())
    }

    // Query the ValueSetter module.
    {
        let query_message = example_value_setter::query::QueryMessage::GetValue;

        let serialized_message = RT::encode_value_setter_query(query_message);
        let module = RT::decode_query(&serialized_message).unwrap();

        let query_response = module.dispatch_query(storage);

        let response: example_value_setter::query::Response =
            serde_json::from_slice(&query_response.response).unwrap();

        assert_eq!(
            response,
            example_value_setter::query::Response {
                value: Some(new_value)
            }
        )
    }
}

#[test]
fn test_demo() {
    run_example()
}
