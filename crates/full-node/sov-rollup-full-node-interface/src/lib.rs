//! Full-node coordination types for the Sovereign SDK.
//!
//! This crate provides types used to coordinate state updates between
//! the STF runner, sequencer, and rollup blueprint. These types are
//! intentionally separated from [`sov_rollup_interface`] to avoid
//! burdening generic rollup-interface consumers with heavy transitive
//! dependencies such as `rockbound` (RocksDB).
//!
//! See the [README](../README.md) for more details on the motivation.

#![deny(missing_docs)]

mod da_sync_state;

pub use da_sync_state::DaSyncState;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::node::SyncStatus;

/// Structure that holds information about the state update that happened in the block.
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
    /// The node's sync status at the time this `StateUpdateInfo` was created.
    pub sync_status: SyncStatus,
}

/// A coordinating channel that manages both a full [`StateUpdateInfo`] channel
/// and a storage-only channel. Consumers that only need the latest storage (e.g. bonding proof
/// services) can subscribe to the storage channel, avoiding a dependency on the full
/// `StateUpdateInfo` type and its transitive dependencies.
///
/// The two channels are updated non-atomically by [`notify`](Self::notify), so their order
/// is not determined. A given consumer should subscribe to only one of the two channels,
/// not both.
pub struct StateChannel<StfState: Clone> {
    state_update_sender: tokio::sync::watch::Sender<StateUpdateInfo<StfState>>,
    storage_sender: tokio::sync::watch::Sender<StfState>,
}

impl<StfState: Clone> StateChannel<StfState> {
    /// Creates a new [`StateChannel`] initialized with the given [`StateUpdateInfo`].
    pub fn new(initial: StateUpdateInfo<StfState>) -> Self {
        let storage = initial.storage.clone();
        let (state_update_sender, _) = tokio::sync::watch::channel(initial);
        let (storage_sender, _) = tokio::sync::watch::channel(storage);
        Self {
            state_update_sender,
            storage_sender,
        }
    }

    /// Returns a receiver for full [`StateUpdateInfo`] updates.
    pub fn subscribe_state_update(
        &self,
    ) -> tokio::sync::watch::Receiver<StateUpdateInfo<StfState>> {
        self.state_update_sender.subscribe()
    }

    /// Returns a receiver for storage-only updates.
    pub fn subscribe_storage(&self) -> tokio::sync::watch::Receiver<StfState> {
        self.storage_sender.subscribe()
    }

    /// Updates both channels with the new state info.
    pub fn notify(&self, info: StateUpdateInfo<StfState>) {
        self.storage_sender.send_replace(info.storage.clone());
        self.state_update_sender.send_replace(info);
    }
}

/// A [`tokio::sync::watch::Receiver`] for [`StateUpdateInfo`].
pub type StateUpdateReceiver<S> = tokio::sync::watch::Receiver<StateUpdateInfo<S>>;
