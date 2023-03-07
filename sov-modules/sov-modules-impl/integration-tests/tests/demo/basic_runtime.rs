//! Basic Runtime Example
//! This `Runtime` serves as a basic example of how to wire up module system and trigger the rollup logic.

#![allow(dead_code)]
use sov_modules_api::{
    mocks::{MockContext, MockPublicKey},
    CallResponse, Context, DispatchCall, DispatchQuery, Error, Genesis, Module,
};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};
use sov_state::{CacheLog, JmtStorage, Storage, ValueReader};
use std::str;

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

struct Message {
    sender: MockPublicKey,
    data: Vec<u8>,
}

fn execute<C: Context<PublicKey = MockPublicKey>>(storage: &C::Storage, message: Message) {
    let module = Runtime::<C>::decode_call(&message.data).unwrap();
    let context = Context::new(message.sender);
    let result = module.dispatch_call(storage.clone(), &context);

    assert!(result.is_ok())
}

fn check_query(query: Vec<u8>, expected_response: &str, storage: JmtStorage) {
    let module = Runtime::<MockContext>::decode_query(&query).unwrap();
    let query_response = module.dispatch_query(storage);

    let response = str::from_utf8(&query_response.response).unwrap();
    assert_eq!(response, expected_response)
}

fn simulate_da() -> Vec<Message> {
    let mut messages = Vec::default();
    messages.extend(CallGenerator::election_call_messages());
    messages.extend(CallGenerator::value_setter_call_messages());
    messages
}

use serial_test::serial;

#[test]
#[serial]
fn test_demo_values_in_cache() {
    type C = MockContext;

    let path = schemadb::temppath::TempPath::new();

    let storage = JmtStorage::with_path(&path).unwrap();
    Runtime::<C>::genesis(storage.clone()).unwrap();

    for message in simulate_da() {
        execute::<C>(&storage, message);
    }

    check_query(
        QueryGenerator::generate_query_election_message(),
        r#"{"Result":{"name":"candidate_2","count":3}}"#,
        storage.clone(),
    );

    check_query(
        QueryGenerator::generate_query_value_setter_message(),
        r#"{"value":99}"#,
        storage,
    );
}

#[test]
#[serial]
fn test_demo_values_in_db() {
    type C = MockContext;

    let path = schemadb::temppath::TempPath::new();

    {
        let mut storage = JmtStorage::with_path(&path).unwrap();
        Runtime::<C>::genesis(storage.clone()).unwrap();

        for message in simulate_da() {
            execute::<C>(&storage, message);
        }

        storage.merge();
        storage.finalize();
    }

    // Generate new storage instance after dumping data to the db.
    {
        let storage = JmtStorage::with_path(path).unwrap();
        check_query(
            QueryGenerator::generate_query_election_message(),
            r#"{"Result":{"name":"candidate_2","count":3}}"#,
            storage.clone(),
        );

        check_query(
            QueryGenerator::generate_query_value_setter_message(),
            r#"{"value":99}"#,
            storage,
        );
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
        for message in simulate_da() {
            execute::<C>(&storage, message);
        }
    }

    // Generate new storage instance after dumping data to the db.
    {
        let storage = JmtStorage::with_path(path).unwrap();

        check_query(
            QueryGenerator::generate_query_election_message(),
            r#"{"Err":"Election is not frozen"}"#,
            storage.clone(),
        );

        check_query(
            QueryGenerator::generate_query_value_setter_message(),
            r#"{"value":null}"#,
            storage,
        );
    }
}

// Test helpers
struct CallGenerator {}

impl CallGenerator {
    fn election_call_messages() -> Vec<Message> {
        let mut messages = Vec::default();

        let admin = MockPublicKey::try_from("admin").unwrap();

        let set_candidates_message =
            example_election::call::CallMessage::<MockContext>::SetCandidates {
                names: vec!["candidate_1".to_owned(), "candidate_2".to_owned()],
            };

        messages.push((admin.clone(), set_candidates_message));

        let voters = vec![
            MockPublicKey::try_from("voter_1").unwrap(),
            MockPublicKey::try_from("voter_2").unwrap(),
            MockPublicKey::try_from("voter_3").unwrap(),
        ];

        for voter in voters {
            let add_voter_message =
                example_election::call::CallMessage::<MockContext>::AddVoter(voter.clone());

            messages.push((admin.clone(), add_voter_message));

            let vote_message = example_election::call::CallMessage::<MockContext>::Vote(1);
            messages.push((voter, vote_message));
        }

        let freeze_message = example_election::call::CallMessage::<MockContext>::FreezeElection;
        messages.push((admin, freeze_message));

        messages
            .into_iter()
            .map(|(sender, m)| Message {
                sender,
                data: Runtime::<MockContext>::encode_election_call(m),
            })
            .collect()
    }

    fn value_setter_call_messages() -> Vec<Message> {
        let admin = MockPublicKey::try_from("admin").unwrap();
        let new_value = 99;
        let set_value_msg = example_value_setter::call::CallMessage::DoSetValue(
            example_value_setter::call::SetValue { new_value },
        );

        vec![Message {
            sender: admin,
            data: Runtime::<MockContext>::encode_value_setter_call(set_value_msg),
        }]
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
