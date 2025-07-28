use sov_chain_state::ChainState;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{
    AccessoryStateReaderAndWriter, AccessoryStateVec, BlockHooks, Context, DaSpec, FinalizeHook,
    GenesisState, Module, ModuleId, ModuleInfo, Spec, StateCheckpoint, StateVec, TxState,
};
use sov_modules_stf_blueprint::Runtime;
use sov_state::Storage;
use sov_test_utils::runtime::genesis::optimistic::HighLevelOptimisticGenesisConfig;
use sov_test_utils::runtime::traits::MinimalGenesis;
use sov_test_utils::TestUser;

mod visible_hash_basic_kernel;
mod visible_hash_soft_confirmations;

mod gas_price_soft_confirmations;

type S = sov_test_utils::TestSpec;

type TestRunner<RT> = sov_test_utils::runtime::TestRunner<RT, S>;

#[derive(ModuleInfo, Clone)]
pub struct TestVisibleHashModule<S: Spec> {
    #[id]
    id: ModuleId,

    #[state]
    finalize_hook_hash: AccessoryStateVec<<S::Storage as Storage>::Root>,

    #[state]
    begin_slot_hash: StateVec<<S::Storage as Storage>::Root>,

    #[module]
    chain_state: ChainState<S>,
}

impl<S: Spec> Module for TestVisibleHashModule<S> {
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

impl<S: Spec> BlockHooks for TestVisibleHashModule<S> {
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

impl<S: Spec> FinalizeHook for TestVisibleHashModule<S> {
    type Spec = S;

    fn finalize_hook(
        &mut self,
        visible_hash: &<S::Storage as Storage>::Root,
        state: &mut impl AccessoryStateReaderAndWriter,
    ) {
        self.finalize_hook_hash
            .push(visible_hash, state)
            .unwrap_infallible();
    }
}

struct TestClosureArgs<S: Storage> {
    prev_finalize_hook_hash: S::Root,
    prev_slot_hash: S::Root,
    finalize_hook_hash: S::Root,
    current_slot_hash: S::Root,
    begin_slot_hash: Option<S::Root>,
}

/// A helper method for the visible hash tests. It advances the module state by `num_slots` and runs a closure with
/// the specified test arguments after each iteration.
fn last_state_root_closure<RT: Runtime<S> + MinimalGenesis<S>>(
    test_closure: &mut impl FnMut(TestClosureArgs<<S as Spec>::Storage>),
    runner: &mut TestRunner<RT>,
    num_slots: u64,
) {
    let module = TestVisibleHashModule::<S>::default();

    let mut prev_finalize_hook_hash = runner.query_visible_state(|state| {
        module
            .finalize_hook_hash
            .last(state)
            .unwrap_infallible()
            .unwrap()
    });

    for _ in 0..num_slots {
        runner.advance_slots(1_usize);

        runner.query_state(|state| {
            let prev_slot_hash = module
                .chain_state
                .last_root(state)
                .unwrap_infallible()
                .unwrap();

            let finalize_hook_hash = module
                .finalize_hook_hash
                .last(state)
                .unwrap_infallible()
                .unwrap();

            let current_slot_hash = *runner.state_root();

            let begin_slot_hash = module.begin_slot_hash.last(state).unwrap_infallible();

            test_closure(TestClosureArgs {
                prev_slot_hash,
                finalize_hook_hash,
                prev_finalize_hook_hash,
                current_slot_hash,
                begin_slot_hash,
            });

            prev_finalize_hook_hash = finalize_hook_hash;
        });
    }
}
