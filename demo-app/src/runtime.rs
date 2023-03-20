use sov_modules_api::{Context, Module};
use sov_modules_macros::{DispatchCall, DispatchQuery, Genesis, MessageCodec};

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
#[derive(Genesis)] //, DispatchCall, DispatchQuery, MessageCodec)]
pub(crate) struct Runtime<C: Context> {
    /// Definition of the first module in the rollup (must implement the sov_modules_api::Module trait).
    #[allow(unused)]
    election: election::Election<C>,
    // Definition of the second module in the rollup (must implement the sov_modules_api::Module trait).
    #[allow(unused)]
    value_setter: value_setter::ValueSetter<C>,
}
