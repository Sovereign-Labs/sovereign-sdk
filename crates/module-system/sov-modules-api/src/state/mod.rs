mod accessors;
mod traits;

#[cfg(test)]
mod tests;

#[cfg(any(feature = "test-utils", feature = "evm"))]
pub use accessors::UnmeteredStateWrapper;
pub use accessors::{
    AccessoryDelta, BootstrapWorkingSet, BorshSerializedSize, ChangeSet, GenesisStateAccessor,
    KernelStateAccessor, PreExecWorkingSet, RevertableTxState, StateCheckpoint,
    StateMetricsProvider, StateProvider, TxChangeSet, TxScratchpad, WorkingSet,
};
#[cfg(feature = "native")]
pub use accessors::{AccessoryStateCheckpoint, ApiStateAccessor, ApiStateAccessorError};
#[cfg(feature = "native")]
use sov_rollup_interface::ProvableHeightTracker;
pub use sov_state::TypeErasedEvent;
#[cfg(feature = "native")]
pub use traits::ProvenStateAccessor;
pub use traits::{
    AccessoryStateReader, AccessoryStateReaderAndWriter, AccessoryStateWriter, GenesisState,
    InfallibleKernelStateAccessor, InfallibleStateAccessor, InfallibleStateReaderAndWriter,
    PerBlockCache, PrivilegedKernelAccessor, ProvableStateReader, ProvableStateWriter,
    StateAccessor, StateAccessorError, StateReader, StateReaderAndWriter, StateWriter, TxState,
    VersionReader,
};

#[cfg(feature = "native")]
/// Utilities to allow tracking the maximum provable height of the rollup.
pub mod provable_height_tracker;
