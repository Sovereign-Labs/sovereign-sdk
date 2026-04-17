use serde::{Deserialize, Serialize};

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
    /// The current height through which we've synced
    pub fn synced_da_height(&self) -> u64 {
        match self {
            SyncStatus::Syncing {
                synced_da_height, ..
            }
            | SyncStatus::Synced { synced_da_height } => *synced_da_height,
        }
    }

    /// The height to which we're syncing.
    pub fn target_da_height(&self) -> u64 {
        match self {
            SyncStatus::Synced { synced_da_height } => *synced_da_height,
            SyncStatus::Syncing {
                target_da_height, ..
            } => *target_da_height,
        }
    }

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
