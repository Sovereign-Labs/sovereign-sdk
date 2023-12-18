//! Module specification definitions.

use core::fmt::Debug;

use borsh::{BorshDeserialize, BorshSerialize};
use digest::typenum::U32;
use digest::Digest;
use sov_rollup_interface::RollupAddress;

use crate::common::{GasUnit, PublicKey, Signature, Witness};
use crate::storage::Storage;

/// The `Spec` trait configures certain key primitives to be used by a by a particular instance of a rollup.
/// `Spec` is almost always implemented on a Context object; since all Modules are generic
/// over a Context, rollup developers can easily optimize their code for different environments
/// by simply swapping out the Context (and by extension, the Spec).
///
/// For example, a rollup running in a STARK-based zkVM like Risc0 might pick Sha256 or Poseidon as its preferred hasher,
/// while a rollup running in an elliptic-curve based SNARK such as `Placeholder` from the =nil; foundation might
/// prefer a Pedersen hash. By using a generic Context and Spec, a rollup developer can trivially customize their
/// code for either (or both!) of these environments without touching their module implementations.
pub trait Spec {
    /// The Address type used on the rollup. Typically calculated as the hash of a public key.
    #[cfg(all(feature = "native", feature = "std"))]
    type Address: RollupAddress
        + BorshSerialize
        + BorshDeserialize
        + Sync
        // Do we always need this, even when the module does not have a JSON
        // Schema? That feels a bit wrong.
        + ::schemars::JsonSchema
        + Into<crate::common::AddressBech32>
        + From<crate::common::AddressBech32>
        + alloc::str::FromStr<Err = anyhow::Error>;

    /// The Address type used on the rollup. Typically calculated as the hash of a public key.
    #[cfg(all(feature = "native", not(feature = "std")))]
    type Address: RollupAddress
        + BorshSerialize
        + BorshDeserialize
        + Sync
        + Into<crate::common::AddressBech32>
        + From<crate::common::AddressBech32>
        + alloc::str::FromStr<Err = anyhow::Error>;

    /// The Address type used on the rollup. Typically calculated as the hash of a public key.
    #[cfg(not(feature = "native"))]
    type Address: RollupAddress + BorshSerialize + BorshDeserialize;

    /// Authenticated state storage used by the rollup. Typically some variant of a merkle-patricia trie.
    type Storage: Storage + Send + Sync;

    /// The public key used for digital signatures
    #[cfg(feature = "native")]
    type PrivateKey: crate::common::PrivateKey<
        PublicKey = Self::PublicKey,
        Signature = Self::Signature,
    >;

    /// The public key used for digital signatures
    #[cfg(all(feature = "native", feature = "std"))]
    type PublicKey: PublicKey + ::schemars::JsonSchema + alloc::str::FromStr<Err = anyhow::Error>;

    /// The public key used for digital signatures
    #[cfg(not(all(feature = "native", feature = "std")))]
    type PublicKey: PublicKey;

    /// The hasher preferred by the rollup, such as Sha256 or Poseidon.
    type Hasher: Digest<OutputSize = U32>;

    /// The digital signature scheme used by the rollup
    #[cfg(all(feature = "native", feature = "std"))]
    type Signature: Signature<PublicKey = Self::PublicKey>
        + alloc::str::FromStr<Err = anyhow::Error>
        + serde::Serialize
        + for<'a> serde::Deserialize<'a>
        + schemars::JsonSchema;

    /// The digital signature scheme used by the rollup
    #[cfg(all(not(all(feature = "native", feature = "std")), not(feature = "serde")))]
    type Signature: Signature<PublicKey = Self::PublicKey>;

    /// The digital signature scheme used by the rollup
    #[cfg(all(not(all(feature = "native", feature = "std")), feature = "serde"))]
    type Signature: Signature<PublicKey = Self::PublicKey>
        + serde::Serialize
        + for<'a> serde::Deserialize<'a>;

    /// A structure containing the non-deterministic inputs from the prover to the zk-circuit
    type Witness: Witness;
}

/// A context contains information which is passed to modules during
/// transaction execution. Currently, context includes the sender of the transaction
/// as recovered from its signature.
///
/// Context objects also implement the [`Spec`] trait, which specifies the types to be used in this
/// instance of the state transition function. By making modules generic over a `Context`, developers
/// can easily update their cryptography to conform to the needs of different zk-proof systems.
pub trait Context: Spec + Clone + Debug + PartialEq + 'static {
    /// Gas unit for the gas price computation.
    type GasUnit: GasUnit;

    /// Sender of the transaction.
    fn sender(&self) -> &Self::Address;

    /// Sequencer of the runtime.
    fn sequencer(&self) -> &Self::Address;

    /// Constructor for the Context.
    fn new(sender: Self::Address, sequencer: Self::Address, height: u64) -> Self;

    /// Returns the height of the current slot as reported by the kernel. This value is
    /// non-decreasing and is guaranteed to be less than or equal to the actual "objective" height of the rollup.
    /// Kernels should ensure that the reported height never falls too far behind the actual height.
    fn slot_height(&self) -> u64;
}
