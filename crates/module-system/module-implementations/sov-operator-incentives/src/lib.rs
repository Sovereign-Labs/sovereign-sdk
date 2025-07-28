mod genesis;
pub use genesis::OperatorIncentivesConfig;
use sov_modules_api::{
    Context, DaSpec, GenesisState, InfallibleStateAccessor, ModuleId, ModuleInfo, ModuleRestApi,
    Spec, StateValue, TxState,
};
mod call;
pub use call::CallMessage;

/// The OperatorIncentives module is responsible for managing incentives in cases where a rollup is secured by an authority.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct OperatorIncentives<S: Spec> {
    /// Id of the module.
    #[id]
    pub id: ModuleId,

    /// The address that will receive rewards for operating the rollup.
    #[state]
    #[rest_api(include)]
    pub reward_address: StateValue<S::Address>,
}

impl<S: Spec> sov_modules_api::Module for OperatorIncentives<S> {
    type Spec = S;

    type Config = OperatorIncentivesConfig<S>;

    type CallMessage = call::CallMessage<S>;

    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        config: &Self::Config,
        state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        self.init_module(config, state)
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            CallMessage::UpdateRewardAddress { new_reward_address } => {
                Ok(self.update_address(new_reward_address, context, state)?)
            }
        }
    }
}

impl<S: Spec> OperatorIncentives<S> {
    pub fn reward_address(&self, state: &mut impl InfallibleStateAccessor) -> S::Address {
        self.reward_address.get(state).unwrap().unwrap()
    }
}
