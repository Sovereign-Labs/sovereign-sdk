use sov_rollup_interface::common::SlotNumber;
#[cfg(feature = "native")]
use sov_rollup_interface::optimistic::BondingProofService;
use sov_rollup_interface::optimistic::{SerializedAttestation, SerializedChallenge};
use sov_rollup_interface::stf::InvalidProofError;
use sov_rollup_interface::zk::aggregated_proof::{
    AggregatedProofPublicData, SerializedAggregatedProof,
};

#[cfg(feature = "native")]
use super::HasKernel;
#[cfg(feature = "native")]
use crate::rest::StateUpdateReceiver;
use crate::{GetGasPrice, SovAttestation, SovStateTransitionPublicData, Spec, Storage, TxState};

/// The `ProofProcessor` capability is responsible for processing proofs inside
/// the stf-blueprint.
///
/// ## Warning
/// The implementation of this trait is coupled with the implementation of the `GasEnforcer`, trait (and all of its dependencies),
/// since the `ProofProcessor` is responsible for charging disbursing gas fees to the prover.
pub trait ProofProcessor<S: Spec> {
    /// The service that generates bonding proofs for attesters.
    #[cfg(feature = "native")]
    type BondingProofService<K: HasKernel<S>>: BondingProofService;

    /// Creates a new [`BondingProofService`] service.
    #[cfg(feature = "native")]
    fn create_bonding_proof_service<K: HasKernel<S>>(
        &self,
        attester_address: <S as Spec>::Address,
        storage: StateUpdateReceiver<<S as Spec>::Storage>,
    ) -> Self::BondingProofService<K>;

    /// Called by the stf once the zk-proof is received.
    #[allow(clippy::type_complexity)]
    fn process_aggregated_proof<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        proof: SerializedAggregatedProof,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<
        (
            AggregatedProofPublicData<S::Address, S::Da, <S::Storage as Storage>::Root>,
            SerializedAggregatedProof,
        ),
        InvalidProofError,
    >;

    /// Called by the stf once the attestation is received.
    fn process_attestation<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        proof: SerializedAttestation,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> Result<SovAttestation<S>, InvalidProofError>;

    /// Called by the stf once the challenge is received.
    fn process_challenge<ST: TxState<S> + GetGasPrice<Spec = S>>(
        &mut self,
        proof: SerializedChallenge,
        rollup_height: SlotNumber,
        prover_address: &S::Address,
        state: &mut ST,
    ) -> anyhow::Result<SovStateTransitionPublicData<S>, InvalidProofError>;
}
