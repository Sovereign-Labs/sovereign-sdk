#![deny(missing_docs)]
#![doc = include_str!("../README.md")]

pub(crate) mod common;
mod config;
pub(crate) mod metrics;
mod rest_api;
mod tx_status;

pub mod preferred;
pub mod standard;
#[cfg(feature = "test-utils")]
pub mod test_stateless;

use std::sync::Arc;

use axum::async_trait;
#[cfg(feature = "test-utils")]
pub use common::StateUpdateNotification;
pub use common::{react_to_state_updates, Sequencer};
pub use config::{SequencerConfig, SequencerKindConfig};
pub use rest_api::SequencerApis;
use serde::Serialize;
use sov_modules_api::capabilities::RollupHeight;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::TxHash;
pub use tx_status::TxStatusManager;

pub use crate::tx_status::TxStatus;

/// The response type to REST API calls that successfully publish a batch.
#[derive(Debug, Clone, Serialize)]
pub struct SubmitBatchReceipt {
    /// All the hashes of the transactions that were successfully included in
    /// the batch.
    pub tx_hashes: Arc<[TxHash]>,
}

/// See [`Sequencer::is_ready`].
#[derive(Debug, Clone, serde::Serialize)]
#[allow(missing_docs)]
pub enum SequencerNotReadyDetails {
    /// The sequencer is still initializing and is not yet ready to accept transactions.
    Startup,
    /// The node is catching up to the chain tip.
    Syncing {
        target_da_height: u64,
        synced_da_height: u64,
    },
    /// The sequencer is waiting for the DA to finalize more blocks.
    WaitingOnDa {
        finalized_slot_number: SlotNumber,
        needed_finalized_slot_number: SlotNumber,
    },
    /// The sequencer is waiting for the blob sender to be ready.
    WaitingOnBlobSender {
        max_concurrent_blobs: usize,
        nb_of_blobs_in_flight: usize,
    },
    /// The sequencer is a preferred sequencer and dropped too far out of sync, and is currently
    /// attempting to recover.
    PreferredSequencerRecovering,
    /// The prefered sequencer has reached the stop height and is no longer accepting transactions.
    PreferredSequencerAtStopHeight {
        height_to_stop_at: RollupHeight,
        current_height: RollupHeight,
    },
    /// The sequencer is running in replica mode and cannot accept transactions.
    ReplicaMode,
}
/// An object-safe interface to the sequencer, which can be used to
/// publish a proof blob to DA.
#[async_trait]
pub trait ProofBlobSender: Send + Sync + 'static {
    /// Publishes a proof blob to DA.
    async fn produce_and_publish_proof_blob(&self, proof_blob: Arc<[u8]>) -> anyhow::Result<()>;
}
