//! Fetches finalized blocks in parallel between requests heights.
use futures::Stream;
use futures_util::stream::FuturesOrdered;
use futures_util::StreamExt;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::{future_or_shutdown, FutureOrShutdownOutput};
use std::pin::Pin;
use std::sync::Arc;

pub(crate) struct BlockFetcher<Da: DaService> {
    da_service: Arc<Da>,
    blocks: tokio::sync::mpsc::Sender<Da::FilteredBlock>,
    start_height: u64,
    last_finalized_height: u64,
    // Defines how many requests can be made concurrently to a target DaService
    bulk_size: u8,
}

impl<Da> BlockFetcher<Da>
where
    Da: DaService,
{
    pub(crate) fn new(
        da_service: Arc<Da>,
        blocks: tokio::sync::mpsc::Sender<Da::FilteredBlock>,
        start_height: u64,
        last_finalized_height: u64,
        bulk_size: u8,
    ) -> Self {
        BlockFetcher {
            da_service,
            blocks,
            start_height,
            last_finalized_height,
            bulk_size,
        }
    }

    #[tracing::instrument(skip_all, fields(block_count = end - start))]
    async fn fetch_blocks_in_range(
        &self,
        start: u64,
        end: u64,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Da::FilteredBlock>> + Send>> {
        let futures: FuturesOrdered<_> = (start..=end)
            .map(|height| {
                let da_service = Arc::clone(&self.da_service);
                async move {
                    da_service
                        .get_block_at(height)
                        .await
                        .map_err(|e| anyhow::anyhow!(e))
                }
            })
            .collect();

        Box::pin(futures)
    }

    pub(crate) async fn run(
        mut self,
        mut shutdown_receiver: tokio::sync::watch::Receiver<()>,
    ) -> anyhow::Result<()> {
        tracing::trace!(
            start = self.start_height,
            last_finalized_height = self.last_finalized_height,
            "Running bulk block fetcher"
        );
        while self.start_height < self.last_finalized_height {
            let start_height = self.start_height;
            let end_height = std::cmp::min(
                start_height + self.bulk_size as u64,
                self.last_finalized_height,
            );

            // Before doing a bunch of concurrent calls to DaService,
            // we want to make sure that results are going to fit into the channel,
            // so we don't have a bunch of in-flight futures being stuck
            let mut permit = match select_with_shutdown(
                // The range is inclusive.
                self.blocks.reserve_many(self.bulk_size as usize + 1),
                &mut shutdown_receiver,
                "reserve space in channel",
            )
            .await
            {
                Some(p) => p?,
                None => {
                    break;
                }
            };

            let start = std::time::Instant::now();

            let block_stream = match select_with_shutdown(
                self.fetch_blocks_in_range(start_height, end_height),
                &mut shutdown_receiver,
                "self.fetch_blocks_in_range()",
            )
            .await
            {
                Some(b) => b,
                None => break,
            };
            let block_stream = block_stream.fuse();
            futures::pin_mut!(block_stream);

            let mut blocks_fetched = 0;

            loop {
                let next_block = select_with_shutdown(
                    block_stream.next(),
                    &mut shutdown_receiver,
                    "block_stream.next()",
                )
                .await;

                match next_block {
                    Some(Some(block_result)) => {
                        let block = block_result?;
                        blocks_fetched += 1;
                        permit
                            .next()
                            .expect("reserved less permits that bulk_size. Bug")
                            .send(block);
                    }
                    Some(None) => {
                        // Stream ended
                        break;
                    }
                    None => {
                        // Shutdown signal received
                        return Ok(());
                    }
                }
            }

            if blocks_fetched == 0 {
                break;
            }

            tracing::trace!(
                start_height,
                end_height,
                time = ?start.elapsed(),
                blocks = blocks_fetched,
                "Fetched blocks"
            );

            self.start_height = end_height + 1;
        }

        tracing::info!("BlockFetcher synced all finalized headers");

        Ok(())
    }
}

async fn select_with_shutdown<F, T>(
    fut: F,
    shutdown_receiver: &mut tokio::sync::watch::Receiver<()>,
    label: &'static str,
) -> Option<T>
where
    F: std::future::Future<Output = T>,
{
    match future_or_shutdown(fut, shutdown_receiver).await {
        FutureOrShutdownOutput::Output(res) => Some(res),
        FutureOrShutdownOutput::Shutdown => {
            tracing::debug!("Shutting down block fetcher at {}", label);
            None
        }
    }
}
