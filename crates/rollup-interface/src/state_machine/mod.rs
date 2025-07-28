//! Defines types, traits, and helpers that are used by the core state-machine of the rollup.
//! Items in this module must be fully deterministic, since they are expected to be executed inside of zkVMs.
pub mod crypto;
pub mod da;
pub mod stf;
pub mod zk;

use borsh::{BorshDeserialize, BorshSerialize};
pub use bytes::{Buf, BufMut, Bytes, BytesMut};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_universal_wallet::schema::UniversalWallet;

use crate::common::{HexHash, SlotNumber};

pub mod optimistic;
pub mod storage;

/// A rollup transaction hash.
pub type TxHash = HexHash;

/// Defines types and traits distinguishing between "native" (full node) and "zk" execution.
///
/// This module uses a combination of a sealed marker trait, unit structs, and an enum to
/// emulate the behavior of a const-generic enum.
pub mod execution_mode {
    use borsh::{BorshDeserialize, BorshSerialize};
    use serde::{Deserialize, Serialize};

    /// Execution modes for the rollup.
    #[derive(
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
        Hash,
        Serialize,
        Deserialize,
        BorshDeserialize,
        BorshSerialize,
        schemars::JsonSchema,
    )]
    #[serde(rename_all = "snake_case")]
    pub enum RuntimeExecutionMode {
        /// Execution inside of a [`Zkvm`](super::zk::Zkvm).
        Zk,
        /// Execution on a full node.
        Native,
        /// Execution on a full node with the ability to generate proofs.
        /// This adds some overhead on top of the [`RuntimeExecutionMode::Native`] mode.
        WitnessGeneration,
    }
    /// Marker trait for execution modes.
    pub trait ExecutionMode:
        super::sealed::Sealed
        + Send
        + Sync
        + 'static
        + Default
        + Serialize
        + serde::de::DeserializeOwned
    {
        /// An enum variant equivalent to the implementing type.
        const EXECUTION_MODE: RuntimeExecutionMode;
    }
    /// A unit struct marking that execution occurs inside of a [`Zkvm`](super::zk::Zkvm).
    #[derive(
        Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
    )]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct Zk;
    impl ExecutionMode for Zk {
        const EXECUTION_MODE: RuntimeExecutionMode = RuntimeExecutionMode::Zk;
    }
    /// A unit struct marking that execution occurs on a full node.
    #[derive(
        Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
    )]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct Native;
    impl ExecutionMode for Native {
        const EXECUTION_MODE: RuntimeExecutionMode = RuntimeExecutionMode::Native;
    }
    /// A unit struct marking that execution generates a witness, adding additional overhead on top of [`Native`] execution.
    #[derive(
        Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema,
    )]
    #[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
    pub struct WitnessGeneration;
    impl ExecutionMode for WitnessGeneration {
        const EXECUTION_MODE: RuntimeExecutionMode = RuntimeExecutionMode::WitnessGeneration;
    }
}

mod sealed {
    use super::execution_mode::{Native, WitnessGeneration, Zk};
    pub trait Sealed {}

    impl Sealed for Zk {}
    impl Sealed for Native {}
    impl Sealed for WitnessGeneration {}
}

/// A marker trait for general addresses.
pub trait BasicAddress:
    Ord
    + core::fmt::Debug
    + core::fmt::Display
    + Send
    + Unpin
    + Sync
    + Clone
    + core::hash::Hash
    + AsRef<[u8]>
    + for<'a> TryFrom<&'a [u8], Error = anyhow::Error>
    + core::str::FromStr<
        Err: core::fmt::Debug + Into<Box<dyn std::error::Error + Send + Sync + 'static>>,
    > + Serialize
    + DeserializeOwned
    + BorshDeserialize
    + BorshSerialize
    + UniversalWallet
    + MaybeArbitrary
    + JsonSchema
    + 'static
{
}

/// Implement the `arbitrary::Arbitrary` trait when the `arbitrary` feature is enabled.
#[cfg(feature = "arbitrary")]
pub trait MaybeArbitrary: for<'a> arbitrary::Arbitrary<'a> {}
#[cfg(feature = "arbitrary")]
impl<T: for<'a> arbitrary::Arbitrary<'a>> MaybeArbitrary for T {}

/// Implement the `arbitrary::Arbitrary` trait when the `arbitrary` feature is enabled.
#[cfg(not(feature = "arbitrary"))]
pub trait MaybeArbitrary {}
#[cfg(not(feature = "arbitrary"))]
impl<T> MaybeArbitrary for T {}

/// A tracker that returns the maximum provable height of the rollup.
pub trait ProvableHeightTracker: Send + Sync + 'static {
    /// Returns the maximum provable height of the rollup.
    fn max_provable_slot_number(&self) -> SlotNumber;
}

/// Structure that holds information about the state update that happened in the block.
#[cfg(feature = "native")]
#[derive(Clone, derive_more::Debug)]
pub struct StateUpdateInfo<StfState> {
    /// The storage following the state update.
    #[debug(skip)]
    pub storage: StfState,
    #[debug(skip)]
    /// The `DeltaReader` associated with the current `LedgerDb`.
    pub ledger_reader: rockbound::cache::delta_reader::DeltaReader,
    /// What the next event number will be after the state update.
    pub next_event_number: u64,
    /// What the next transaction number will be after the state update.
    pub next_tx_number: u64,
    /// The slot number of the rollup following the state update.
    pub slot_number: SlotNumber,
    /// The latest slot number that was finalized.
    pub latest_finalized_slot_number: SlotNumber,
}
