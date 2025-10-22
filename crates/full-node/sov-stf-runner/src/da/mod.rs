#![allow(dead_code)]
pub(crate) mod bulk_finalized_fetcher;
mod finalized_header_provider;

use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::da::{DaService, SlotData};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc::Receiver;
use tracing::{info_span, Instrument};

// With the block size up to 10 MB, it should fit into 32 GB of RAM.
const MAX_BLOCKS: usize = 1_000;

/// Wrapper around [`DaService`] that optimizes interaction with actual DA:
///  * Pre-fetches all finalized blocks at startup, optimizing sync speed
///  * Providing access to last finalized and head block headers without actual network call
///  * Storing N recent headers to reduce network calls
#[derive(Debug, Clone)]
pub struct DaServiceWithCache<Da: DaService> {
    da_service: Arc<Da>,
    blocks: Receiver<Da::FilteredBlock>,
    start_height: u64,
    prefetch_up_to: u64,
    last_finalized: tokio::sync::watch::Receiver<<Da::Spec as DaSpec>::BlockHeader>,
    is_background_running: std::sync::Arc<AtomicBool>,
    recent_headers: Arc<tokio::sync::RwLock<BTreeMap<u64, <Da::Spec as DaSpec>::BlockHeader>>>,
    // Params:
    // polling
    // total_timeout
    //
}

// Management impl
impl<Da: DaService> DaServiceWithCache<Da> {
    pub async fn new() -> Self {
        todo!()
    }
}

// Usage impl
impl<Da: DaService> DaServiceWithCache<Da> {
    /// Returns the current cached last finalized block header.
    ///
    /// # Errors
    ///
    /// Returns an error if the background polling task has stopped.
    pub fn get_last_finalized(&self) -> anyhow::Result<<Da::Spec as DaSpec>::BlockHeader> {
        if !self.is_background_running.load(Ordering::Acquire) {
            anyhow::bail!("DA header provider background task has stopped");
        }
        Ok(self.last_finalized.borrow().clone())
    }

    /// Fetches a block header at the specified height from the DA service.
    ///
    /// This is a direct passthrough to the underlying DA service.
    pub async fn get_block_header_at(
        &self,
        height: u64,
    ) -> Result<<Da::Spec as DaSpec>::BlockHeader, Da::Error> {
        // TODO: Check chache!
        self.da_service.get_block_header_at(height).await
    }

    // Wrapper around [`DaService::get_block_at`] that checks pre-fetched blocks for the match.
    #[tracing::instrument(skip(self))]
    pub async fn get_block_at(&mut self, height: u64) -> Result<Da::FilteredBlock, Da::Error> {
        if height > self.prefetch_up_to || height < self.start_height {
            tracing::trace!(
                height,
                start_height = self.start_height,
                last_finalized_height = self.prefetch_up_to,
                "Requested height is outside of pre-fetched range, querying DaService directly"
            );
            // TODO: We can also check last finalized height here and don't do re-org aware call
            // TODO: Start using this one:
            // return crate::da_utils::fetch_block_reorg_aware(
            //     self.da_service.as_ref(),
            //     self.sync_state.as_ref(),
            //     height,
            //     self.da_polling_interval,
            //     self.da_total_timeout,
            // ).await
            return self.da_service.get_block_at(height).await;
        }

        let span = info_span!("recv_channel_blocks");
        let block_opt = async {
            while let Some(block) = self.blocks.recv().await {
                let block_height = block.header().height();
                self.start_height = block_height;
                if block_height == height {
                    return Some(block);
                }
                tracing::warn!(
                    block_header = %block.header().display(),
                    "Skipping pre-fetched block from the channel. Reading out of order might've been occurred"
                );
            }
            None
        }
            .instrument(span)
            .await;

        if let Some(block) = block_opt {
            Ok(block)
        } else {
            tracing::info!(
                height,
                "Didn't find block in pre-fetched when it should've been, calling DaService"
            );
            self.da_service.get_block_at(height).await
        }
    }
}
