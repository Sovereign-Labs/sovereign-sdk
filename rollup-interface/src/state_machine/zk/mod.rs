//! Defines the traits that must be implemented by zkVMs. A ZKVM like Risc0 consists of two components,
//! a "guest" and a "host". The guest is the zkVM program itself, and the host is the physical machine on
//! which the zkVM is running. Both the guest and the host are required to implement the [`Zkvm`] trait, in
//! addition to the specialized [`ZkvmGuest`] and [`ZkvmHost`] trait which is appropriate to that environment.
//!
//! For a detailed example showing how to implement these traits, see the
//! [risc0 adapter](https://github.com/Sovereign-Labs/sovereign-sdk/tree/main/adapters/risc0)
//! maintained by the Sovereign Labs team.
use core::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
use digest::Digest;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::da::DaSpec;
use crate::RollupAddress;

/// A trait implemented by the prover ("host") of a zkVM program.
pub trait ZkvmHost: Zkvm {
    /// The associated guest type
    type Guest: ZkvmGuest;
    /// Give the guest a piece of advice non-deterministically
    fn add_hint<T: Serialize>(&self, item: T);

    /// Simulate running the guest using the provided hints.
    ///
    /// Provides a simulated version of the guest which can be
    /// accessed in the current process.
    fn simulate_with_hints(&mut self) -> Self::Guest;

    /// Run the guest in the true zk environment using the provided hints.
    ///
    /// This runs the guest binary compiled for the ZKVM target, optionally
    /// creating a SNARK of correct execution. Running the true guest binary comes
    /// with some mild performance overhead and is not as easy to debug as [`simulate_with_hints`](ZkvmHost::simulate_with_hints).
    fn run(&mut self, with_proof: bool) -> Result<(), anyhow::Error>;
}

/// A Zk proof system capable of proving and verifying arbitrary Rust code
/// Must support recursive proofs.
pub trait Zkvm {
    /// A commitment to the zkVM program which is being proven
    type CodeCommitment: Matches<Self::CodeCommitment>
        + Clone
        + Debug
        + Serialize
        + DeserializeOwned;

    /// The error type which is returned when a proof fails to verify
    type Error: Debug + From<std::io::Error>;

    /// Interpret a sequence of a bytes as a proof and attempt to verify it against the code commitment.
    /// If the proof is valid, return a reference to the public outputs of the proof.
    fn verify<'a>(
        serialized_proof: &'a [u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<&'a [u8], Self::Error>;

    /// Same as [`verify`](Zkvm::verify), except that instead of returning the output
    /// as a serialized array, it returns a state transition structure.
    /// TODO: specify a deserializer for the output
    fn verify_and_extract_output<
        Add: RollupAddress,
        Da: DaSpec,
        Root: Serialize + DeserializeOwned,
    >(
        serialized_proof: &[u8],
        code_commitment: &Self::CodeCommitment,
    ) -> Result<StateTransition<Da, Add, Root>, Self::Error>;
}

/// A trait which is accessible from within a zkVM program.
pub trait ZkvmGuest: Zkvm {
    /// Obtain "advice" non-deterministically from the host
    fn read_from_host<T: DeserializeOwned>(&self) -> T;
    /// Add a public output to the zkVM proof
    fn commit<T: Serialize>(&self, item: &T);
}

/// This trait is implemented on the struct/enum which expresses the validity condition
pub trait ValidityCondition:
    Serialize
    + DeserializeOwned
    + BorshDeserialize
    + BorshSerialize
    + Debug
    + Clone
    + Copy
    + PartialEq
    + Send
    + Sync
    + Eq
{
    /// The error type returned when two [`ValidityCondition`]s cannot be combined.
    type Error: Into<anyhow::Error>;
    /// Combine two conditions into one (typically run inside a recursive proof).
    /// Returns an error if the two conditions cannot be combined
    fn combine<H: Digest>(&self, rhs: Self) -> Result<Self, Self::Error>;
}

/// The public output of a SNARK proof in Sovereign, this struct makes a claim that
/// the state of the rollup has transitioned from `initial_state_root` to `final_state_root`
/// if and only if the condition `validity_condition` is satisfied.
///
/// The period of time covered by a state transition proof may be a single slot, or a range of slots on the DA layer.
#[derive(Clone, Debug, Serialize, Deserialize, BorshSerialize, BorshDeserialize, PartialEq, Eq)]
pub struct StateTransition<Da: DaSpec, Address, Root> {
    /// The state of the rollup before the transition
    pub initial_state_root: Root,
    /// The state of the rollup after the transition
    pub final_state_root: Root,
    /// The slot hash of the state transition
    pub slot_hash: Da::SlotHash,

    /// Rewarded address: the account that has produced the transition proof.
    pub rewarded_address: Address,

    /// An additional validity condition for the state transition which needs
    /// to be checked outside of the zkVM circuit. This typically corresponds to
    /// some claim about the DA layer history, such as (X) is a valid block on the DA layer
    pub validity_condition: Da::ValidityCondition,
}

/// This trait expresses that a type can check a validity condition.
pub trait ValidityConditionChecker<Condition: ValidityCondition>:
    BorshDeserialize + BorshSerialize + Debug
{
    /// The error type returned when a [`ValidityCondition`] is invalid.
    type Error: Into<anyhow::Error>;
    /// Check a validity condition
    fn check(&mut self, condition: &Condition) -> Result<(), Self::Error>;
}

/// A trait expressing that two items of a type are (potentially fuzzy) matches.
/// We need a custom trait instead of relying on [`PartialEq`] because we allow fuzzy matches.
pub trait Matches<T> {
    /// Check if two items are a match
    fn matches(&self, other: &T) -> bool;
}
