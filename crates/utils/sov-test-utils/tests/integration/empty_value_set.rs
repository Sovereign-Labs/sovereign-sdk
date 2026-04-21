//! End-to-end reproduction of "zero-byte values missing after query_visible_state".
//!
//! The SDK-layer tests in `sov-modules-api/tests/integration/state_tests/namespaces.rs`
//! write via a bare `StateCheckpoint` and read back via another bare `StateCheckpoint`.
//! That proves the raw `StateMap` + storage round-trip works for `V = ()`, but skips
//! every layer that sits between a real rollup's `Module::genesis` and an RPC-style
//! `ApiStateAccessor` read. This file fills that gap.
//!
//! Each test builds a minimal runtime that includes `EmptyValueSetModule` alongside the
//! usual test runtime modules, writes a fixed set of keys in the module's `genesis`,
//! and then queries the same module via `TestRunner::query_visible_state` — the path
//! that was observed to return `None` in production.

use schemars::JsonSchema;
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{
    Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, Spec, StateMap, TxState,
};
use sov_paymaster::{Paymaster, PaymasterConfig, SafeVec};
use sov_test_utils::generate_optimistic_runtime;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::TestRunner;
use sov_test_utils::TestSpec;
use sov_value_setter::{ValueSetter, ValueSetterConfig};

/// A test module that holds a `StateMap<u32, ()>` and a `StateMap<u32, bool>`. Both
/// are populated at genesis from the module's config. The `bool` map is a control —
/// if the `()` map loses entries across commit but the `bool` map keeps them, the
/// difference is specifically about zero-byte values.
#[derive(Clone, ModuleInfo)]
pub struct EmptyValueSetModule<S: Spec> {
    #[id]
    pub id: ModuleId,

    /// Map with the zero-byte value — the subject of the investigation.
    #[state]
    pub unit_keys: StateMap<u32, ()>,

    /// Control map whose value type serializes to a single non-zero byte.
    #[state]
    pub bool_keys: StateMap<u32, bool>,

    #[phantom]
    _phantom: std::marker::PhantomData<S>,
}

#[derive(
    Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct EmptyValueSetConfig {
    /// Keys to insert into `unit_keys` at genesis.
    pub unit_keys: Vec<u32>,
    /// Keys to insert into `bool_keys` at genesis (each is set to `false`).
    pub bool_keys: Vec<u32>,
}

#[derive(Debug, PartialEq, Clone, JsonSchema)]
#[serialize(Borsh, Serde)]
pub enum EmptyValueSetEvent {
    /// Placeholder — the test module never emits events.
    Noop,
}

#[derive(Debug, PartialEq, Eq, Clone, JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
pub enum EmptyValueSetCall {
    /// Placeholder — the test module accepts no call messages.
    Noop,
}

impl<S: Spec> Module for EmptyValueSetModule<S> {
    type Spec = S;
    type Config = EmptyValueSetConfig;
    type CallMessage = EmptyValueSetCall;
    type Event = EmptyValueSetEvent;
    type Error = anyhow::Error;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        for key in &config.unit_keys {
            self.unit_keys.set(key, &(), state)?;
        }
        for key in &config.bool_keys {
            self.bool_keys.set(key, &false, state)?;
        }
        Ok(())
    }

    fn call(
        &mut self,
        _msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

generate_optimistic_runtime!(EmptyValueSetRuntime <=
    value_setter: ValueSetter<S>,
    paymaster: Paymaster<S>,
    empty_value_set: EmptyValueSetModule<S>
);

type S = TestSpec;
type RT = EmptyValueSetRuntime<S>;

fn setup(unit_keys: Vec<u32>, bool_keys: Vec<u32>) -> TestRunner<RT, S> {
    let genesis_config =
        HighLevelOptimisticGenesisConfig::generate().add_accounts_with_default_balance(1);

    let admin = genesis_config
        .additional_accounts()
        .first()
        .unwrap()
        .clone();

    let value_setter_config = ValueSetterConfig {
        admin: admin.address(),
    };
    let paymaster_config = PaymasterConfig {
        payers: SafeVec::new(),
    };
    let empty_value_set_config = EmptyValueSetConfig {
        unit_keys,
        bool_keys,
    };

    let genesis = GenesisConfig::from_minimal_config(
        genesis_config.into(),
        value_setter_config,
        paymaster_config,
        empty_value_set_config,
    );

    TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default())
}

#[test]
fn test_unit_value_survives_genesis_commit_and_is_visible_via_api_accessor() {
    let unit_keys = vec![0x12345678u32, 42, 100];
    let runner = setup(unit_keys.clone(), vec![]);

    runner.query_visible_state(|state| {
        let module = EmptyValueSetModule::<S>::default();
        for key in &unit_keys {
            assert_eq!(
                module.unit_keys.get(key, state).unwrap_infallible(),
                Some(()),
                "genesis-written unit key {key} should be visible via query_visible_state",
            );
        }
        assert_eq!(
            module.unit_keys.get(&999u32, state).unwrap_infallible(),
            None,
            "an unwritten key should be absent — sanity check",
        );
    });
}

#[test]
fn test_bool_control_value_survives_genesis_commit_and_is_visible_via_api_accessor() {
    let bool_keys = vec![0x12345678u32, 42, 100];
    let runner = setup(vec![], bool_keys.clone());

    runner.query_visible_state(|state| {
        let module = EmptyValueSetModule::<S>::default();
        for key in &bool_keys {
            assert_eq!(
                module.bool_keys.get(key, state).unwrap_infallible(),
                Some(false),
                "control: bool-valued key {key} should be visible via query_visible_state",
            );
        }
    });
}
