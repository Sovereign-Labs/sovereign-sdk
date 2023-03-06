//! Basic Runtime Example
//! This `Runtime` serves as a basic example of how to wire up module system and trigger the rollup logic.

#![allow(dead_code)]

use example_election::Candidate;
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    CallResponse, Context, DispatchCall, DispatchQuery, Error, Genesis, Module,
};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::{CacheLog, JmtStorage, Storage, ValueReader};

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

fn call_election_module<C: Context<PublicKey = MockPublicKey>>(storage: &C::Storage) {
    let sender = MockPublicKey::try_from("admin").unwrap();
    let admin_context = C::new(sender);

    // Election module
    // Send candidates
    {
        let set_candidates_message = example_election::call::CallMessage::<C>::SetCandidates {
            names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
        };

        let serialized_message = Runtime::<C>::encode_election_call(set_candidates_message);
        let module = Runtime::<C>::decode_call(&serialized_message).unwrap();
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

            let serialized_message = Runtime::<C>::encode_election_call(add_voter_message);
            let module = Runtime::<C>::decode_call(&serialized_message).unwrap();

            let result = module.dispatch_call(storage.clone(), &admin_context);

            assert!(result.is_ok())
        }
    }

    // Vote
    {
        for voter in voters {
            let voter_context = C::new(voter);
            let vote_message = example_election::call::CallMessage::<C>::Vote(1);

            let serialized_message = Runtime::<C>::encode_election_call(vote_message);
            let module = Runtime::<C>::decode_call(&serialized_message).unwrap();

            let result = module.dispatch_call(storage.clone(), &voter_context);
            assert!(result.is_ok())
        }
    }

    // Freeze
    {
        let freeze_message = example_election::call::CallMessage::<C>::FreezeElection;

        let serialized_message = Runtime::<C>::encode_election_call(freeze_message);
        let module = Runtime::<C>::decode_call(&serialized_message).unwrap();

        let result = module.dispatch_call(storage.clone(), &admin_context);
        assert!(result.is_ok())
    }
}

fn call_value_setter_module<C: Context<PublicKey = MockPublicKey>>(storage: &C::Storage) {
    let sender = MockPublicKey::try_from("admin").unwrap();

    let admin_context = C::new(sender);

    // Set new value
    let new_value = 99;
    {
        let set_value_msg = example_value_setter::call::CallMessage::DoSetValue(
            example_value_setter::call::SetValue { new_value },
        );

        let serialized_message = Runtime::<C>::encode_value_setter_call(set_value_msg);
        let module = Runtime::<C>::decode_call(&serialized_message).unwrap();
        let result = module.dispatch_call(storage.clone(), &admin_context);

        assert!(result.is_ok())
    }
}

fn query_election_returns_correct_result(storage: JmtStorage) -> bool {
    let serialized_message = QueryGenerator::generate_query_election_message();
    let module = Runtime::<MockContext>::decode_query(&serialized_message).unwrap();

    let query_response = module.dispatch_query(storage);

    let response: example_election::query::Response =
        serde_json::from_slice(&query_response.response).unwrap();

    response
        == example_election::query::Response::Result(Some(Candidate {
            name: "candidate_2".to_owned(),
            count: 3,
        }))
}

fn query_value_setter_returns_correct_result(storage: JmtStorage) -> bool {
    let serialized_message = QueryGenerator::generate_query_value_setter_message();
    let module = Runtime::<MockContext>::decode_query(&serialized_message).unwrap();

    let new_value = 99;
    let query_response = module.dispatch_query(storage);
    let response: example_value_setter::query::Response =
        serde_json::from_slice(&query_response.response).unwrap();

    response
        == example_value_setter::query::Response {
            value: Some(new_value),
        }
}

fn check_query(storage: JmtStorage) -> bool {
    query_election_returns_correct_result(storage.clone())
        && query_value_setter_returns_correct_result(storage)
}

use serial_test::serial;
#[test]
#[serial]
fn test_demo_values_in_cache() {
    type C = MockContext;

    let path = schemadb::temppath::TempPath::new();
    let storage = JmtStorage::with_path(path).unwrap();
    // Initialize the rollup: Call genesis on the Runtime
    Runtime::<C>::genesis(storage.clone()).unwrap();

    call_election_module::<C>(&storage);
    call_value_setter_module::<C>(&storage);
    // We didn't save anything in the db, but they exist in the Storage cache.
    assert!(check_query(storage))
}

#[test]
#[serial]
fn test_demo_values_in_db() {
    type C = MockContext;
    let path = schemadb::temppath::TempPath::new();
    {
        let mut storage = JmtStorage::with_path(&path).unwrap();

        Runtime::<C>::genesis(storage.clone()).unwrap();

        call_election_module::<C>(&storage);
        call_value_setter_module::<C>(&storage);
        // Save storage values in the db.
        storage.merge();
        storage.finalize();
    }
    // Generate new storage instance after dumping data to the db.
    {
        let storage = JmtStorage::with_path(path).unwrap();
        assert!(check_query(storage))
    }
}

#[test]
#[serial]
fn test_demo_values_not_in_db() {
    type C = MockContext;
    let path = schemadb::temppath::TempPath::new();
    {
        let storage = JmtStorage::with_path(&path).unwrap();

        Runtime::<C>::genesis(storage.clone()).unwrap();

        call_election_module::<C>(&storage);
        call_value_setter_module::<C>(&storage);
        // Don't save anything in the db.
    }
    // The DB lookup fails because we generated fresh storage, but we didn't save values in the db before.
    {
        let storage = JmtStorage::with_path(path).unwrap();
        assert!(!check_query(storage))
    }
}

struct QueryGenerator {}

impl QueryGenerator {
    fn generate_query_election_message() -> Vec<u8> {
        let query_message = example_election::query::QueryMessage::GetResult;
        Runtime::<MockContext>::encode_election_query(query_message)
    }

    fn generate_query_value_setter_message() -> Vec<u8> {
        let query_message = example_value_setter::query::QueryMessage::GetValue;
        Runtime::<MockContext>::encode_value_setter_query(query_message)
    }
}
