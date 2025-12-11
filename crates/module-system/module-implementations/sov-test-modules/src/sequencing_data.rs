use borsh::{BorshDeserialize, BorshSerialize};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::{Context, DaSpec, GenesisState, Module, ModuleId, ModuleInfo, Spec, TxState};

#[derive(Clone, ModuleInfo)]
pub struct SequencingDataTester<S: Spec> {
    #[id]
    pub id: ModuleId,
    #[phantom]
    _phantom: std::marker::PhantomData<S>,
}

#[derive(
    Clone,
    BorshSerialize,
    BorshDeserialize,
    PartialEq,
    Eq,
    Debug,
    Hash,
    schemars::JsonSchema,
    UniversalWallet,
)]
pub enum CallMessage {
    /// Captures the sequencing data from context and checks it's correctness
    Main,
}

impl<S: Spec> Module for SequencingDataTester<S> {
    type Spec = S;

    type Config = ();
    type CallMessage = CallMessage;
    type Event = ();
    type Error = anyhow::Error;

    fn genesis(
        &mut self,
        _genesis_rollup_header: &<<S as Spec>::Da as DaSpec>::BlockHeader,
        _config: &Self::Config,
        _state: &mut impl GenesisState<S>,
    ) -> Result<(), Self::Error> {
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
