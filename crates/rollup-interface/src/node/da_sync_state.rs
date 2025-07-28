use std::sync::atomic::AtomicU64;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

/// The node sync status tracker
#[derive(Debug)]
pub struct DaSyncState {
    /// Last processed DA height.
    pub synced_da_height: AtomicU64,
    /// Latest known DA height.
    pub target_da_height: AtomicU64,
    /// The sender of the sync status
    pub sync_status_sender: watch::Sender<SyncStatus>,
}

/// The status of the current sync
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncStatus {
    /// The node has caught up to the chain tip
    Synced {
        /// The current height through which we've synced
        synced_da_height: u64,
    },
    /// The node is currently syncing
    Syncing {
        /// The current height through which we've synced
        synced_da_height: u64,
        /// The height to which we're syncing. This reflects the current view of the DA chain tip
        target_da_height: u64,
    },
}

impl SyncStatus {
    /// Where a node starts syncing from.
    pub const START: Self = SyncStatus::Syncing {
        synced_da_height: 0,
        target_da_height: 0,
    };

    /// Returns true if the sync status is `Synced`
    pub fn is_synced(&self) -> bool {
        match self {
            SyncStatus::Synced { .. } => true,
            SyncStatus::Syncing { .. } => false,
        }
    }

    /// Distance to the chain head.
    pub fn distance(&self) -> u64 {
        match self {
            SyncStatus::Synced { .. } => 0,
            SyncStatus::Syncing {
                synced_da_height,
                target_da_height,
            } => target_da_height.saturating_sub(*synced_da_height),
        }
    }
}

impl DaSyncState {
    /// Updates the target height of the sync state.
    pub fn update_target(
        &self,
        target_da_height: u64,
    ) -> Result<(), watch::error::SendError<SyncStatus>> {
        self.target_da_height
            .store(target_da_height, std::sync::atomic::Ordering::Release);

        self.sync_status_sender.send(self.status())
    }

    /// Updates the synced height of the sync state.
    pub fn update_synced(&self, synced_da_height: u64) {
        self.synced_da_height
            .store(synced_da_height, std::sync::atomic::Ordering::Release);

        self.target_da_height
            .fetch_update(
                std::sync::atomic::Ordering::AcqRel,
                std::sync::atomic::Ordering::Acquire,
                |current_target| {
                    if current_target < synced_da_height {
                        Some(synced_da_height)
                    } else {
                        None
                    }
                },
            )
            .ok();

        if let Err(e) = self.sync_status_sender.send(self.status()) {
            tracing::warn!(
                "Failed to send sync status update after updating synced height: {:?}. There are no receivers for the sync status.",
                e
            );
        }
    }

    /// Latest known sync status.
    pub fn status(&self) -> SyncStatus {
        let current = self
            .synced_da_height
            .load(std::sync::atomic::Ordering::Acquire);
        let target = self
            .target_da_height
            .load(std::sync::atomic::Ordering::Acquire);

        if current == target {
            SyncStatus::Synced {
                synced_da_height: current,
            }
        } else {
            SyncStatus::Syncing {
                synced_da_height: current,
                target_da_height: target,
            }
        }
    }
}
