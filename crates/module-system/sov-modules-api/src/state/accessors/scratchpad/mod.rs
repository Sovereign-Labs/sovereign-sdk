//! Transaction state management structures.
//!
//! This module contains types for managing state during transaction execution:
//! - [`RevertableTxState`]: Temporary state changes within a transaction that can be committed or reverted
//! - [`LayeredRevertableTxState`]: Multi-layered revertable state to avoid unbounded recursion
//! - [`TxScratchpad`]: Transaction-level state diff without gas metering
//! - [`PreExecWorkingSet`]: Pre-execution working set with gas metering for checks
//! - [`WorkingSet`]: Full transaction execution context with gas metering and events
//! - [`TxChangeSet`]: Records of changes from transaction execution

mod layered_revertable_tx_state;
mod pre_exec_working_set;
mod revertable_tx_state;
mod tx_scratchpad;
mod working_set;

pub use layered_revertable_tx_state::LayeredRevertableTxState;
pub use pre_exec_working_set::PreExecWorkingSet;
pub use revertable_tx_state::RevertableTxState;
pub use tx_scratchpad::{TxChangeSet, TxScratchpad};
pub use working_set::WorkingSet;
