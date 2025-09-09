//! Runtime module definitions.

use core::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
use sov_rollup_interface::da::DaSpec;
use sov_state::EventContainer;
use sov_universal_wallet::schema::UniversalWallet;

use crate::common::ModuleError;
use crate::{GenesisState, ModuleId, TxState};

mod dispatch;
mod event;
mod gas_spec;
mod prefix;
mod spec;

pub use dispatch::*;
pub use event::*;
pub use gas_spec::*;
pub use prefix::*;
pub use spec::*;

/// The core trait implemented by all modules. This trait defines how a module is initialized at genesis,
/// and how it handles user transactions (if applicable).
pub trait Module: Clone {
    /// Execution context.
    type Spec: Spec;

    /// Configuration for the genesis method.
    type Config;

    /// Module defined argument to the call method.
    type CallMessage: Debug
        + BorshSerialize
        + BorshDeserialize
        + UniversalWallet
        + schemars::JsonSchema
        + Clone
        + PartialEq
        + Eq;

    /// Module defined event resulting from a call method.
    type Event: Debug
        + BorshSerialize
        + BorshDeserialize
        + schemars::JsonSchema
        + 'static
        + core::marker::Send
        + PartialEq;

    /// Genesis is called once when a rollup is deployed.
    ///
    /// You should use this function to initialize all of your module's `StateValue`s and run any other
    /// one-time setup. Since this function runs only once, it's perfectly acceptible to do expensive operations
    /// here. Note that your function should still be deterministic, however.
    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<Self::Spec as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<Self::Spec>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    /// `call` accepts a `CallMessage` and executes it, changing the state of the module and emitting events. `Context` contains useful
    /// information including the transaction's sender and sequencer.
    ///
    /// ## Gas Metering and Charging
    /// The SDK will automatically meter and charge for consumption of storage resources (getting and setting data).
    /// In the overwhelming majority of cases, your call message should be able to rely exclusively on this mechanism
    /// without needing to manually charge gas. However, if your callmessage consumes a significant amount of compute or memory
    /// that does *not* correlate with its consumption of storage resources, you may need to manually charge gas using the [`Module::charge_gas`] method.
    ///
    /// ## Determinism
    /// Your call method should be fully deterministic, both in the content and the order of its state changes/events. You MUST not rely on network access
    /// or random number generation in your call method, since these are inherently non-deterministic. You should
    /// also take great care when iterating over `HashMap`s, since iteration order is not guaranteed.
    ///
    /// ## The "Native" Feature Flag
    /// The "native" feature flag is used to gate off code that is *not* executed when generating zk proofs of your rollup.
    ///
    /// A common pattern you'll see in the SDK is the use of the `native` feature flag to conditionally execute code.
    /// This is especially useful when you want to compute some data that is not an essential part
    /// of the state transition. For example, you might maintain a secondary index of all addresses
    /// which hold a certain token and use it to serve API queries, but not allow access to it onchain.
    /// Your module should always generate the same state changes (excluding "AccessoryState") regardless of the feature flag.
    /// Note that events are only emitted if the `native` feature flag is enabled, and are *not* queryable onchain.
    fn call(
        &mut self,
        _message: Self::CallMessage,
        _context: &Context<Self::Spec>,
        _state: &mut impl TxState<Self::Spec>,
    ) -> anyhow::Result<()>;

    /// Attempts to charge the provided amount of gas from the working set reverting the transaction if unsuccessful.
    ///
    /// The amount of funds to charge will be computed using the current gas price.
    ///
    /// # Errors
    /// Returns an error if charging gas fails due to running out of gas, or due to an overflow computing the scalar value.
    fn charge_gas(
        &self,
        state: &mut impl TxState<Self::Spec>,
        gas: &<Self::Spec as Spec>::Gas,
    ) -> anyhow::Result<()> {
        Ok(state.charge_gas(gas)?)
    }
}

/// A [`Module`] that has a well-defined and known [JSON
/// Schema](https://json-schema.org/) for its [`Module::CallMessage`].
///
/// This trait is intended to support code generation tools, CLIs, and
/// documentation. This trait is blanket-implemented for all modules with a
/// [`Module::CallMessage`] associated type that implements
/// [`schemars::JsonSchema`].
pub trait ModuleCallJsonSchema: Module {
    /// Returns the JSON schema for [`Module::CallMessage`].
    fn json_schema() -> String;
}

impl<T> ModuleCallJsonSchema for T
where
    T: Module,
    T::CallMessage: schemars::JsonSchema,
{
    fn json_schema() -> String {
        let schema = ::schemars::schema_for!(T::CallMessage);

        serde_json::to_string_pretty(&schema)
            .expect("Failed to serialize JSON schema; this is a bug in the module")
    }
}

/// Every module has to implement this trait, but it is not designed to be done by hand. Use the `ModuleInfo` macro to derive it.
pub trait ModuleInfo {
    /// Execution context.
    type Spec: Spec;

    /// Returns id of the module.
    fn id(&self) -> &ModuleId;

    /// Returns the prefix of the module.
    fn prefix(&self) -> ModulePrefix;

    /// Returns addresses of all the other modules this module is dependent on
    fn dependencies(&self) -> Vec<&ModuleId>;

    /// Returns true if the call is safe to submit on behalf of an aribtrary 3rd party as far
    /// as this module is concerned. The provided `Any` *should* be an instance of the module's `CallMessage`
    ///
    /// This is an advanced function of the SDK. Types which are not safe include any `CallMessage`
    /// where the action of sequencing them is used to signify implicit permission from the
    /// sequencer to change some settings or perform some action; sequencers should utilise their
    /// discretion in accepting these calls (e.g. by only accepting them from whitelisted senders).
    fn is_safe_for_sequencer(
        &self,
        _call: InnerEnumVariant<'_>,
        _sequencer_address: &<<Self::Spec as Spec>::Da as DaSpec>::Address,
    ) -> bool {
        true
    }
}

/// Allows modules to emit events. Events are served via the REST API but are *not* included in zk proofs.
pub trait EventEmitter: ModuleInfo {
    /// Execution context.
    type Spec: Spec;
    /// Module defined event resulting from a call method.
    type Event: Debug + BorshSerialize + BorshDeserialize + 'static + core::marker::Send;

    /// Emits an event with an auto-generated event key composed by the module
    /// of origin's name and the `enum` variant's name of the event.
    #[allow(unused_variables)]
    fn emit_event(&self, state: &mut impl EventContainer, event: Self::Event) {
        if cfg!(feature = "native") {
            let key = event_key(self.prefix().module_name(), &event);
            state.add_event(&key, event);
        }
    }

    /// Emits an event with a custom event key.
    #[allow(unused_variables)]
    fn emit_event_with_custom_key(
        &self,
        state: &mut impl EventContainer,
        event_key: &str,
        event: Self::Event,
    ) {
        if cfg!(feature = "native") {
            state.add_event(event_key, event);
        }
    }
}

fn event_key<T: Debug>(module_prefix: &str, event: &T) -> String {
    let event_debug = format!("{event:?}");
    // Hacky logic to get the first identifier in the Debug representation.
    let event_identifier = event_debug
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .next()
        .unwrap_or_default();

    format!("{module_prefix}/{event_identifier}")
}

/// A helper function to make string representation of the inner call message from the runtime's `Decodable` representation.
pub fn call_message_repr<D: DispatchCall>(decodable: &D::Decodable) -> String {
    // Extract the variant name from the part right after the first parenthesis
    let dbg = format!("{decodable:?}");
    let variant_part = dbg
        .split_once('(')
        .map(|(_idx, after_parent)| {
            let end_idx = after_parent
                .find([' ', '{', '('])
                .unwrap_or(after_parent.len());
            &after_parent[..end_idx]
        })
        .unwrap_or("Unknown");

    format!("{}_{}", decodable.discriminant().as_ref(), variant_part)
}

impl<T> EventEmitter for T
where
    T: ModuleInfo + Module,
{
    type Spec = <T as ModuleInfo>::Spec;
    type Event = <T as Module>::Event;
}

/// A trait that specifies how a runtime should encode the data for each module
pub trait EncodeCall<M: Module>: DispatchCall {
    /// The encoding function
    fn encode_call(data: M::CallMessage) -> Vec<u8> {
        <Self as DispatchCall>::encode(&Self::to_decodable(data))
    }

    /// Converts the module call message into the [`DispatchCall::Decodable`] type
    fn to_decodable(data: M::CallMessage) -> Self::Decodable;
}

/// Allows a module to initialize its state once during rollup deployment.
pub trait Genesis {
    /// Execution context of the module.
    type Spec: Spec;

    /// Initial configuration for the module.
    type Config;

    /// Initializes the state of the rollup.
    fn genesis(
        &mut self,
        genesis_rollup_header: &<<Self::Spec as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<Self::Spec>,
    ) -> Result<(), ModuleError>;
}

impl<T> Genesis for T
where
    T: Module,
{
    type Spec = <Self as Module>::Spec;

    type Config = <Self as Module>::Config;

    fn genesis(
        &mut self,
        genesis_rollup_header: &<<Self::Spec as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<Self::Spec>,
    ) -> Result<(), ModuleError> {
        Ok(<Self as Module>::genesis(
            self,
            genesis_rollup_header,
            config,
            state,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_keys_struct() {
        #[derive(Debug)]
        struct Struct1;
        assert_eq!(event_key("module", &Struct1), "module/Struct1".to_string());

        #[derive(Debug)]
        struct Struct2 {}
        assert_eq!(
            event_key("module", &Struct2 {}),
            "module/Struct2".to_string()
        );

        #[derive(Debug)]
        struct Struct3 {
            #[allow(dead_code)]
            foo: u32,
        }
        assert_eq!(
            event_key("module", &Struct3 { foo: 1 }),
            "module/Struct3".to_string()
        );

        #[derive(Debug)]
        struct Struct4();
        assert_eq!(
            event_key("module", &Struct4()),
            "module/Struct4".to_string()
        );

        #[derive(Debug)]
        struct Struct5(#[allow(dead_code)] u32);
        assert_eq!(
            event_key("module", &Struct5(1)),
            "module/Struct5".to_string()
        );
    }

    #[test]
    fn event_keys_enum() {
        #[derive(Debug)]
        enum Enum1 {
            Variant1,
        }
        assert_eq!(
            event_key("module", &Enum1::Variant1),
            "module/Variant1".to_string()
        );

        #[derive(Debug)]
        enum Enum2 {
            Variant1(#[allow(dead_code)] u32),
        }
        assert_eq!(
            event_key("module", &Enum2::Variant1(1)),
            "module/Variant1".to_string()
        );

        #[derive(Debug)]
        enum Enum3 {
            Variant1 {
                #[allow(dead_code)]
                a: u32,
            },
        }
        assert_eq!(
            event_key("module", &Enum3::Variant1 { a: 1 }),
            "module/Variant1".to_string()
        );
    }
}
