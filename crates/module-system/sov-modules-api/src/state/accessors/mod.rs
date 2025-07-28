//! Defines traits for storage access

use std::marker::PhantomData;

use internals::RevertableWriter;
use sov_state::Namespace;

mod access_controls;
mod checkpoints;
mod genesis;
mod internals;

#[cfg(test)]
mod tests;
#[cfg(any(feature = "test-utils", feature = "evm"))]
mod unmetered_state_wrapper;

pub use temp_cache::TempCache;
#[cfg(any(feature = "test-utils", feature = "evm"))]
pub use unmetered_state_wrapper::UnmeteredStateWrapper;

#[cfg(feature = "native")]
mod http_api;

#[cfg(feature = "native")]
pub use http_api::{ApiStateAccessor, ApiStateAccessorError};

mod scratchpad;

mod kernel;
mod temp_cache;

#[cfg(feature = "native")]
pub use checkpoints::native::AccessoryStateCheckpoint;
pub use checkpoints::{ChangeSet, StateCheckpoint};
pub use genesis::GenesisStateAccessor;
pub use internals::AccessoryDelta;
pub use kernel::{BootstrapWorkingSet, KernelStateAccessor};
pub use scratchpad::{PreExecWorkingSet, RevertableTxState, TxChangeSet, TxScratchpad, WorkingSet};
pub use temp_cache::BorshSerializedSize;

use self::seal::UniversalStateAccessor;
use super::traits::PerBlockCache;
use super::{StateReaderAndWriter, VersionReader};
use crate::Spec;

pub(super) mod seal {
    use sov_state::{Namespace, SlotKey, SlotValue};

    /// A helper trait allowing a type to access any namespace by their *runtime* enum variant.
    pub trait UniversalStateAccessor {
        fn get_size(&mut self, namespace: Namespace, key: &SlotKey) -> Option<u32>;

        fn get_value(&mut self, namespace: Namespace, key: &SlotKey) -> Option<SlotValue>;

        fn set_value(&mut self, namespace: Namespace, key: &SlotKey, value: SlotValue);

        fn delete_value(&mut self, namespace: Namespace, key: &SlotKey);
    }
}

/// A state abstraction that can be used to kickstart transaction execution.
///
/// See [`StateCheckpoint`], which is the canonical implementation of this
/// trait.
///
/// This is a **sealed trait**.
pub trait StateProvider<S: Spec>:
    Sized
    + UniversalStateAccessor
    + StateReaderAndWriter<sov_state::User>
    + StateReaderAndWriter<sov_state::Kernel>
    + VersionReader
    + PerBlockCache
{
    /// Transforms this [`StateProvider`] into a [`TxScratchpad`].
    fn to_tx_scratchpad(self) -> TxScratchpad<S, Self>;
}

impl<S: Spec> StateProvider<S> for StateCheckpoint<S> {
    fn to_tx_scratchpad(self) -> TxScratchpad<S, StateCheckpoint<S>> {
        TxScratchpad {
            inner: RevertableWriter::new(self),
            phantom: PhantomData,
        }
    }
}
