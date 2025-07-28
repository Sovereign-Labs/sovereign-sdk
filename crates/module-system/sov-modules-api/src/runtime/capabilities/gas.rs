use std::convert::Infallible;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_rollup_interface::da::DaSpec;
use sov_state::{Kernel, User};

use crate::transaction::{AuthenticatedTransactionData, ProverReward, RemainingFunds};
use crate::{
    Amount, Context, Gas, InfallibleStateAccessor, OperatingMode, Rewards, Spec, StateAccessor,
    StateReader, StateWriter,
};

/// Enforces gas limits and penalties for transactions.
///
/// ## Warning
/// The implementation of this trait is coupled with the implementation of the `SequencerRemuneration`, trait and the behavior
/// of the `BlobSelector` (which may reserve gas for blob serialization/deserialization).
pub trait GasEnforcer<S: Spec> {
    /// Checks that the transaction has enough gas to be processed.
    ///
    /// ## Note
    /// This method has to reserve enough gas to cover the pre-execution checks cost of the transaction.
    /// If the transaction doesn't have enough gas to cover the pre-execution checks, the method should return an error.
    ///
    /// ## Behavior
    /// This function **should** charge the transaction sender for the gas locked in the transaction because his balance
    /// may change during the transaction execution.
    #[allow(clippy::result_large_err)]
    fn try_reserve_gas(
        &mut self,
        tx: &AuthenticatedTransactionData<S>,
        gas_price: &<S::Gas as Gas>::Price,
        ctx: &mut Context<S>,
        state: &mut impl StateAccessor,
    ) -> anyhow::Result<()>;

    /// Checks that the proof or attestation has enough gas to be processed.
    ///
    /// ## Note
    /// This method has to reserve enough gas to cover the pre-execution checks cost of the transaction.
    /// If the transaction doesn't have enough gas to cover the pre-execution checks, the method should return an error.
    ///
    /// ## Behavior
    /// This function **should** charge the transaction sender for the gas locked in the transaction because his balance
    /// may change during the transaction execution.
    #[allow(clippy::result_large_err)]
    fn try_reserve_gas_for_proof(
        &mut self,
        tx: &AuthenticatedTransactionData<S>,
        gas_price: &<S::Gas as Gas>::Price,
        sender: &S::Address,
        scratchpad: &mut impl StateAccessor,
    ) -> anyhow::Result<()>;

    /// Rewards the prover
    /// This method should not fail.
    ///
    /// ## Correctness note
    /// The caller of this method must ensure that sufficient funds are reserved.  
    /// If there are not enough funds reserved, the method will panic.
    fn reward_prover(
        &mut self,
        prover_rewards: &ProverReward,
        oprating_mode: OperatingMode,
        tx_scratchpad: &mut impl InfallibleStateAccessor,
    );

    /// Refunds any remaining gas to the payer after the transaction is processed.
    /// This method should not fail.
    ///
    /// ## Correctness note
    /// The caller of this method must ensure that sufficient funds are reserved.  
    /// If there are not enough funds reserved, the method will panic.
    fn refund_remaining_gas(
        &mut self,
        sender: &S::Address,
        remaining_funds: &RemainingFunds,
        tx_scratchpad: &mut impl InfallibleStateAccessor,
    );

    /// The sequencer refunds the prover for the authentication of the transactions.
    /// This method is unmetered, so implementers MUST ensure that its cost is small.
    /// This is not difficult to do - in general, this method should simply do a token transfer
    /// between known addresses.
    ///
    /// ## Warnings
    /// - The implementation of this method is coupled with the implementation of the `SequencerRemuneration`, trait.
    /// - This method is not metered, so be careful about using expensive operations.
    fn reward_prover_from_sequencer_balance(
        &mut self,
        funds_used: Amount,
        sequencer: &S::Address,
        oprating_mode: OperatingMode,
        tx_scratchpad: &mut impl InfallibleStateAccessor,
    ) -> anyhow::Result<()>;

    /// Returns the remaining funds escrowed from pre-execution checks to the sequencer.
    ///
    /// ## Warnings
    /// - The implementation of this method is coupled with the implementation of the `SequencerRemuneration`, trait.
    /// - This method is not metered, so be careful about using expensive operations.
    fn return_escrowed_funds_to_sequencer<
        Accessor: StateReader<Kernel, Error = Infallible>
            + StateWriter<Kernel, Error = Infallible>
            + StateWriter<User, Error = Infallible>
            + StateReader<User, Error = Infallible>,
    >(
        &mut self,
        initial_escrow: Amount,
        reward: Rewards,
        sequencer: &<S::Da as DaSpec>::Address,
        tx_scratchpad: &mut Accessor,
    );
}

/// A structure that contains block gas information.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, BorshSerialize, BorshDeserialize)]
#[serde(bound = "GU: DeserializeOwned")]
pub struct BlockGasInfo<GU: Gas> {
    /// The gas limit of the block execution.
    /// This value is dynamically adjusted over time to account for the increase
    /// in proving/execution performance.
    gas_limit: GU,
    /// The gas used by the block execution.
    /// This value is set to zero at the beginning of the block execution (in the [`ChainState::synchronize_chain`] capability),
    /// and is populated once the block execution is complete.
    gas_used: GU,
    /// The base fee per gas used for the block execution. This value combined with the `gas_used`
    /// can be used to compute the total base fee (expressed in gas tokens) paid by the block execution.
    base_fee_per_gas: GU::Price,
}

impl<GU: Gas> BlockGasInfo<GU> {
    /// Creates a new [`BlockGasInfo`] with the provided gas limit and base fee per gas.
    /// The `gas_used` is set to zero.
    pub fn new(gas_limit: GU, base_fee_per_gas: GU::Price) -> Self {
        Self {
            gas_limit,
            gas_used: GU::zero(),
            base_fee_per_gas,
        }
    }

    /// Creates a new [`BlockGasInfo`] with the provided gas limit and base fee per gas.
    pub fn with_usage(gas_limit: GU, base_fee_per_gas: GU::Price, gas_used: GU) -> Self {
        Self {
            gas_limit,
            gas_used,
            base_fee_per_gas,
        }
    }

    /// Updates the gas used by the block execution.
    pub fn update_gas_used(&mut self, gas_used: GU) {
        self.gas_used = gas_used;
    }

    /// Returns the gas limit of the block execution.
    pub const fn gas_limit(&self) -> &GU {
        &self.gas_limit
    }

    /// Returns the gas used by the block execution.
    pub const fn gas_used(&self) -> &GU {
        &self.gas_used
    }

    /// Returns the base fee per gas used for the block execution.
    pub const fn base_fee_per_gas(&self) -> &GU::Price {
        &self.base_fee_per_gas
    }
}
