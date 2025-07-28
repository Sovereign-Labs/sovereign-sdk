use sov_modules_api::macros::serialize;
use sov_modules_api::{Amount, Spec};
#[derive(Debug, PartialEq, Clone, derive_more::Display)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
/// Reasons for slashing a prover
pub enum SlashingReason {
    /// The proof is not a valid zk-proof - ie the verifier did not accept the proof.
    ProofInvalid,

    /// The genesis hash supplied is incorrect
    IncorrectGenesisHash,

    /// The initial state root contained in the [`sov_modules_api::AggregatedStateTransition`] outputs is incorrect
    IncorrectInitialStateRoot,

    /// The initial transition slot contained in the [`sov_modules_api::AggregatedStateTransition`] has no associated transition
    /// in the chain state module.
    InitialTransitionDoesNotExist,

    /// The initial slot hash contained in the [`sov_modules_api::AggregatedStateTransition`] outputs is incorrect
    IncorrectInitialSlotHash,

    /// The initial slot number is greater than or equal to the final slot number
    InvalidSlotNumbers,

    /// The final transition slot contained in the [`sov_modules_api::AggregatedStateTransition`] has no associated transition
    /// in the chain state module.
    FinalTransitionDoesNotExist,

    /// The final state root contained in the [`sov_modules_api::AggregatedStateTransition`] outputs is incorrect
    IncorrectFinalStateRoot,

    /// The final slot hash contained in the [`sov_modules_api::AggregatedStateTransition`] outputs is incorrect
    IncorrectFinalSlotHash,
}

#[derive(Debug, PartialEq, Clone)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
/// The reasons for penalizing a prover
pub enum PenalizationReason {
    /// We penalize the prover for submitting a proof for transitions that have already been processed
    ProofAlreadyProcessed,
}

#[derive(Debug, PartialEq, Clone, schemars::JsonSchema)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
#[schemars(bound = "S: Spec", rename = "Event")]
/// Events for prover incentives
pub enum Event<S: Spec> {
    /// The prover has been bonded. The deposit is the amount of the bond and the total balance is the total amount staked.
    Registered {
        /// The address of the prover that was bonded.
        prover: S::Address,
        /// The amount deposited by the prover for bond.
        amount: Amount,
    },
    /// A sequencer deposited funds to stake.
    Deposited {
        /// The address of the sequencer that was deposited to.
        prover: S::Address,
        /// The amount of the deposit.
        deposit: Amount,
    },

    /// The prover has been unbonded. The amount withdrawn is the amount of the bond that was withdrawn.
    Exited {
        /// The address of the prover that was unbonded.
        prover: S::Address,
        /// The amount that was withdrawn from the provers bond.
        amount_withdrawn: Amount,
    },

    /// Event for processing a valid proof
    ProcessedValidProof {
        /// The address of the prover that submitted a proof that was processed and determined to
        /// be valid.
        prover: S::Address,
        /// The amount the prover was rewarded for submitting a valid proof.
        reward: Amount,
    },
}
