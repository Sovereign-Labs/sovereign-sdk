#[cfg(feature = "native")]
use sov_accounts::query::{AccountsRpcImpl, AccountsRpcServer};
#[cfg(feature = "native")]
use sov_bank::query::{BankRpcImpl, BankRpcServer};
#[cfg(feature = "native")]
use sov_election::query::{ElectionRpcImpl, ElectionRpcServer};
#[cfg(feature = "native")]
#[cfg(feature = "experimental")]
use sov_evm::query::{EvmRpcImpl, EvmRpcServer};
pub use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Context;
#[cfg(feature = "native")]
use sov_modules_macros::{cli_parser, expose_rpc};
use sov_modules_macros::{DefaultRuntime, DispatchCall, Genesis, MessageCodec};
#[cfg(feature = "native")]
use sov_sequencer_registry::query::{SequencerRegistryRpcImpl, SequencerRegistryRpcServer};
#[cfg(feature = "native")]
use sov_value_setter::query::{ValueSetterRpcImpl, ValueSetterRpcServer};

/// The Rollup entrypoint.
///
/// On a high level, the rollup node receives serialized call messages from the DA layer and executes them as atomic transactions.
/// Upon reception, the message has to be deserialized and forwarded to an appropriate module.
///
/// The module specific logic is implemented by module creators, but all the glue code responsible for message
/// deserialization/forwarding is handled by a rollup `runtime`.
///
/// In order to define the runtime we need to specify all the modules supported by our rollup (see the `Runtime` struct bellow)
///
/// The `Runtime` together with associated interfaces (`Genesis`, `DispatchCall`, `MessageCodec`)
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
/// `#[derive(MessageCodec)` adds deserialization capabilities to the `Runtime` (implements `decode_call` method).
/// `Runtime::decode_call` accepts serialized call message and returns a type that implements the `DispatchCall` trait.
///  The `DispatchCall` implementation (derived by a macro) forwards the message to the appropriate module and executes its `call` method.
///
/// Similar mechanism works for queries with the difference that queries are submitted by users directly to the rollup node
/// instead of going through the DA layer.

#[cfg_attr(
    feature = "native",
    cli_parser(DefaultContext),
    expose_rpc(DefaultContext)
)]
#[derive(Genesis, DispatchCall, MessageCodec, DefaultRuntime)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub struct Runtime<C: Context> {
    pub bank: sov_bank::Bank<C>,
    pub sequencer_registry: sov_sequencer_registry::SequencerRegistry<C>,
    pub election: sov_election::Election<C>,
    pub value_setter: sov_value_setter::ValueSetter<C>,
    pub accounts: sov_accounts::Accounts<C>,
    #[cfg(feature = "experimental")]
    pub evm: sov_evm::Evm<C>,
}
