use sov_modules_api::{Context, Module, ModuleInfo};
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
///

#[derive(Genesis, DispatchCall, DispatchQuery, MessageCodec)]
#[serialization(borsh::BorshDeserialize, borsh::BorshSerialize)]
pub(crate) struct Runtime<C: Context> {
    #[allow(unused)]
    election: election::Election<C>,

    #[allow(unused)]
    value_setter: value_setter::ValueSetter<C>,

    #[allow(unused)]
    accounts: accounts::Accounts<C>,
}
// 1. Generate GenesisConfig
// 2. Impl Genesis with GenesisConfig = GenesisConfig
// 3. Add new_config(..) to RT //OR Genesis

pub struct GenesisConfig<C: Context> {
    election: <election::Election<C> as Module>::Config,
    value_setter: <value_setter::ValueSetter<C> as Module>::Config,
    accounts: <accounts::Accounts<C> as Module>::Config,
}

impl<C: Context> GenesisConfig<C> {
    fn new() -> Self {
        Self {
            election: election::NoConfig,
            value_setter: value_setter::NoConfig,
            accounts: accounts::NoConfig,
        }
    }
}

//impl<C: Context> RuntimeConfig for C {
//   type GenesisConfig = GenesisConfig<C>;
// /}

// TODO add macro to generate the following code.
impl<C: Context> Runtime<C> {
    pub(crate) fn new() -> Self {
        Self {
            election: election::Election::new(),
            value_setter: value_setter::ValueSetter::new(),
            accounts: accounts::Accounts::new(),
        }
    }
}

/*
// 1. Add Config to Genesis
// 2. Implement Conf in modules
// 3. Add #config in RT
// 4. Handle #config in macros

trait Gen {
    fn rt_gen<T, Config: AsRef<T>>(&self, config: Config);
}

trait Mod {
    type Config;

    fn genesis(&self, config: &Self::Config);
}

struct ModA {}

impl Mod for ModA {
    type Config = ();

    fn genesis(&self, config: &Self::Config) {
        todo!()
    }
}

struct ModB {}

impl Mod for ModB {
    type Config = String;

    fn genesis(&self, config: &Self::Config) {
        todo!()
    }
}

struct RT {
    moda: ModA,
    modb: ModB,
}

#[derive(Clone)]
struct Config {
    config_a: <ModA as Mod>::Config,
    config_b: <ModB as Mod>::Config,
}

fn new_config() -> Config {
    Config {
        config_a: todo!(),
        config_b: todo!(),
    }
}

impl AsRef<<ModA as Mod>::Config> for Config {
    fn as_ref(&self) -> &<ModA as Mod>::Config {
        &self.config_a
    }
}

impl AsRef<<ModB as Mod>::Config> for Config {
    fn as_ref(&self) -> &<ModB as Mod>::Config {
        &self.config_b
    }
}

impl Gen for RT {
    fn rt_gen<Config>(&self, config: Config) {
        self.moda.genesis(config.as_ref());
        self.modb.genesis(config.as_ref());
    }
}

trait X<K> {
    fn foo(k: K);
}

struct S {}

impl X<String> for S {
    fn foo(k: String) {
        todo!()
    }
}
*/
