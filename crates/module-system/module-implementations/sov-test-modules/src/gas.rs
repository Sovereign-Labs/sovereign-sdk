/// A module for testing gas charges
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_modules_api::macros::{config_value, UniversalWallet};
use sov_modules_api::{
    Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi, Spec, StateValue,
    TxState,
};

/// A message to set a value
#[derive(
    Clone,
    BorshSerialize,
    BorshDeserialize,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
    UniversalWallet,
)]
pub enum CallMessage {
    /// Sets a value
    SetValue {
        #[allow(missing_docs)]
        value: u64,
    },
}

/// A new module:
/// - Must derive `ModuleInfo`
/// - Must contain `[id]` field
/// - Can contain any number of ` #[state]` or `[module]` fields
/// - Can derive ModuleRestApi to automatically generate Rest API endpoints
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct GasTester<S: Spec> {
    /// The ID of the module.
    #[id]
    pub id: ModuleId,

    /// A state value
    #[state]
    pub value: StateValue<u64>,

    #[phantom]
    _phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for GasTester<S> {
    type Spec = S;

    type Config = ();

    type CallMessage = CallMessage;

    type Event = ();

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    fn call(
        &mut self,
        msg: Self::CallMessage,
        _context: &Context<Self::Spec>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        match msg {
            CallMessage::SetValue { value } => {
                self.charge_gas(
                    state,
                    &S::Gas::from(config_value!("EXAMPLE_CUSTOM_GAS_PRICE")),
                )?;

                self.value.set(&value, state)?;

                Ok(())
            }
        }
    }
}
