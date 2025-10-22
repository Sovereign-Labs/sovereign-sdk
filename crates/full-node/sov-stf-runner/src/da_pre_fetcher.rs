//! Utilities for parallel fetching of the finalized blocks.

use std::pin::Pin;
use std::sync::Arc;

use crate::da::bulk_finalized_fetcher::BlockFetcher;
use futures::stream::FuturesOrdered;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::{DaService, SlotData};
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use tokio::sync::mpsc::Receiver;
use tracing::{info_span, Instrument as _};

// With the block size up to 10 MB, it should fit into 32 GB of RAM.
const MAX_BLOCKS: usize = 1_000;

/// Service that pre-fetcher blocks from given start height up to last finalized height at the moment of construction.
/// After that it proxies all requests to underlying DaService.
pub struct FinalizedBlocksBulkFetcher<Da: DaService> {
    da_service: Arc<Da>,
    blocks: Receiver<Da::FilteredBlock>,
    start_height: u64,
    pub(crate) last_finalized_height: u64,
}

impl<Da> FinalizedBlocksBulkFetcher<Da>
where
    Da: DaService,
{
    pub async fn new(
        da_service: Arc<Da>,
        start_height: u64,
        bulk_size: u8,
        shutdown_receiver: tokio::sync::watch::Receiver<()>,
    ) -> anyhow::Result<(Self, tokio::task::JoinHandle<anyhow::Result<()>>)> {
        let (blocks_sender, blocks_receiver) = tokio::sync::mpsc::channel(MAX_BLOCKS);

        let last_finalized_height = da_service
            .get_last_finalized_block_header()
            .await
            .map_err(|e| anyhow::anyhow!(e))?
            .height();

        let block_fetcher = BlockFetcher::new(
            da_service.clone(),
            blocks_sender,
            start_height,
            last_finalized_height,
            bulk_size,
        );

        let background_handle = tokio::spawn(async {
            // Intentionally swallow error to not produce panic on shutdown.
            match block_fetcher.run(shutdown_receiver).await {
                Ok(()) => {
                    tracing::debug!("BlockFetcher task has completed");
                }
                Err(e) => {
                    tracing::error!(error = ?e, "BlockFetcher task has failed");
                }
            };
            Ok(())
        });

        Ok((
            Self {
                da_service,
                blocks: blocks_receiver,
                start_height,
                last_finalized_height,
            },
            background_handle,
        ))
    }

    /// Wrapper around [`DaService::get_block_at`]
    #[tracing::instrument(skip(self))]
    pub async fn get_block_at(&mut self, height: u64) -> Result<Da::FilteredBlock, Da::Error> {
        if height > self.last_finalized_height || height < self.start_height {
            tracing::trace!(
                height,
                start_height = self.start_height,
                last_finalized_height = self.last_finalized_height,
                "Requested height is outside of pre-fetched range, querying DaService directly"
            );
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

#[cfg(test)]
mod tests {
    use sov_mock_da::storable::StorableMockDaService;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn check_that_all_blocks_are_collected_instant_finality() -> anyhow::Result<()> {
        let da_service = StorableMockDaService::new_in_memory(Default::default(), 0).await;
        let blocks_number = 200;
        for i in 1..=blocks_number {
            da_service.send_transaction(&[i; 32]).await.await??;
        }

        let (sender, mut receiver) = tokio::sync::watch::channel(());
        receiver.mark_unchanged();

        let (mut fetcher, handle) =
            FinalizedBlocksBulkFetcher::new(Arc::new(da_service), 0, 10, receiver).await?;

        for i in 0..blocks_number {
            let block = fetcher.get_block_at(i as u64).await?;
            assert_eq!(i as u64, block.header().height());
        }

        // pre-fetcher might exit by that point.
        let _ = sender.send(());
        handle.await??;
        Ok(())
    }
}
