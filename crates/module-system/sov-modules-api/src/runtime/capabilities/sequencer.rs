use std::convert::Infallible;
use std::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::DaSpec;
use sov_state::{Kernel, User};

use crate::transaction::SequencerReward;
use crate::{Amount, InfallibleStateAccessor, Spec, StateReader, StateWriter};

/// An known sequencer for a rollup.
#[derive(Debug, Clone, Serialize, Deserialize, BorshSerialize, BorshDeserialize, Eq, PartialEq)]
#[serde(bound = "S::Address: serde::Serialize + serde::de::DeserializeOwned")]
pub struct AllowedSequencer<S: Spec> {
    /// The rollup address of the sequencer.
    pub address: S::Address,
    /// The staked balance of the sequencer.
    pub balance: Amount,
}

/// Authorizes the sequencer to submit and process batches.
pub trait SequencerAuthorization<S: Spec> {
    /// Authorize the preferred sequencer based on their current balance.
    fn is_preferred_sequencer(
        &self,
        sequencer: &<<S as Spec>::Da as DaSpec>::Address,
        state: &mut impl InfallibleStateAccessor,
    ) -> bool;
}

/// Functionality related to the rewarding and slashing of the sequencer.
///
/// ## Warning
/// The implementation of this trait is coupled with the implementation of the `GasEnforcer`, trait and the behavior
/// of the `BlobSelector` (which may reserve gas for blob serialization/deserialization).
pub trait SequencerRemuneration<S: Spec> {
    /// Reward the sequencer for correctly processing the forced registration.
    /// If the registration was reverted, refund the sequencer rollup address.
    ///
    /// ## Warnings
    /// - The implementation of this method is coupled with the implementation of the `GasEnforcer`, trait.
    /// - This method is not metered, so be careful about using expensive operations.
    fn reward_sequencer_or_refund<
        Accessor: StateReader<Kernel, Error = Infallible>
            + StateWriter<Kernel, Error = Infallible>
            + StateWriter<User, Error = Infallible>
            + StateReader<User, Error = Infallible>,
    >(
        &mut self,
        sequencer: &<S::Da as DaSpec>::Address,
        sequencer_rollup_address: &S::Address,
        reward: SequencerReward,
        state: &mut Accessor,
    );

    /// Gets the address of the preferred sequencer, if one exists.
    fn preferred_sequencer(
        &self,
        scratchpad: &mut impl InfallibleStateAccessor,
    ) -> Option<<S::Da as DaSpec>::Address>;
}
