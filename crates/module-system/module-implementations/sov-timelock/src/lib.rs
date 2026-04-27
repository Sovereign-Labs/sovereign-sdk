//! Timelock proposal management module.

#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

use sov_modules_api::capabilities::{
    ProposalId, TimelockCapability, TimelockError, TimelockPolicy,
};
use sov_modules_api::{
    Context, DaSpec, Error, GenesisState, Module, ModuleId, ModuleInfo, ModuleRestApi,
    NotInstantiable, Spec, TxState,
};

/// Timelock module.
///
/// The initial MVP exposes the timelock capability interface without storing proposals.
#[derive(Clone, ModuleInfo, ModuleRestApi)]
pub struct Timelock<S: Spec> {
    /// The ID of the sov-timelock module.
    #[id]
    pub id: ModuleId,

    #[phantom]
    phantom: std::marker::PhantomData<S>,
}

impl<S: Spec> Module for Timelock<S> {
    type Spec = S;

    type Config = ();

    type CallMessage = NotInstantiable;

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
        _context: &Context<S>,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Self::Error> {
        unreachable!()
    }
}

impl<S: Spec> TimelockCapability<S> for Timelock<S> {
    fn has_proposal(
        &self,
        _address: &S::Address,
        _proposal_id: &ProposalId,
        _state: &mut impl TxState<S>,
    ) -> Result<bool, Error> {
        Ok(false)
    }

    fn register_proposal(
        &mut self,
        _address: &S::Address,
        _proposal_id: ProposalId,
        _policy: TimelockPolicy,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn try_unlock_proposal(
        &mut self,
        _address: &S::Address,
        _proposal_id: &ProposalId,
        _state: &mut impl TxState<S>,
    ) -> Result<(), Error> {
        Err(TimelockError::ProposalNotFound.into())
    }
}
