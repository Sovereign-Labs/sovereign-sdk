use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{
    BlockHooks, Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, Spec, StateCheckpoint,
    StateVec, TxState,
};
use sov_state::Storage;
use sov_test_utils::generate_zk_runtime;
use sov_test_utils::runtime::genesis::zk::config::HighLevelZkGenesisConfig;

use crate::hooks_derive::{TestRunner, S};

#[derive(ModuleInfo, Clone)]
pub struct CorrectHooksOverride<S: Spec> {
    #[id]
    id: ModuleId,

    #[state]
    begin_slot_hash: StateVec<<S::Storage as Storage>::Root>,
}

impl<S: Spec> Module for CorrectHooksOverride<S> {
    type Spec = S;
    type Config = ();
    type CallMessage = ();
    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<S::Da as DaSpec>::BlockHeader,

        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn call(
        &mut self,
        _msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        _state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}

impl<S: Spec> BlockHooks for CorrectHooksOverride<S> {
    type Spec = S;

    fn begin_rollup_block_hook(
        &mut self,
        visible_hash: &<<S as Spec>::Storage as Storage>::Root,
        state: &mut StateCheckpoint<Self::Spec>,
    ) {
        self.begin_slot_hash
            .push(visible_hash, state)
            .unwrap_infallible();
    }
}

generate_zk_runtime!(CorrectHooksRuntime <= hooks: CorrectHooksOverride<S>);

type RT = CorrectHooksRuntime<S>;

fn setup() -> TestRunner<RT> {
    let genesis_config = HighLevelZkGenesisConfig::generate();

    let genesis = GenesisConfig::from_minimal_config(genesis_config.into(), ());

    TestRunner::new_with_genesis(genesis.into_genesis_params(), RT::default())
}

/// The hook override succeeds if the module implements the [`BlockHooks`] trait for every spec.
#[test]
fn hook_override_succeeds_when_generic_override() {
    let mut runner = setup();

    runner.advance_slots(1);

    runner.query_state(|state| {
        assert_eq!(
            CorrectHooksOverride::<S>::default()
                .begin_slot_hash
                .len(state)
                .unwrap_infallible(),
            1,
            "The hooks should have ran"
        );
    });
}
