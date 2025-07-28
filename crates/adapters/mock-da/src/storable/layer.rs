//! Data Availability layer is a single entry to all available blocks.

use std::ops::Range;

use rand::prelude::{SliceRandom, SmallRng};
use rand::{Rng, SeedableRng};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};
use sha2::Digest;
use sov_rollup_interface::common::{HexHash, HexString};
use tokio::sync::{broadcast, watch};

use crate::config::{GENESIS_BLOCK, GENESIS_HEADER};
use crate::storable::entity;
use crate::storable::entity::blobs::Entity as Blobs;
use crate::storable::entity::block_headers::Entity as BlockHeaders;
use crate::storable::entity::{blobs, block_headers, finalized_height, query_last_saved_block};
use crate::{
    MockAddress, MockBlob, MockBlock, MockBlockHeader, MockDaConfig, MockHash,
    RandomizationBehaviour, RandomizationConfig,
};

/// Struct that stores blobs and block headers. Controller of the sea orm entities.
#[derive(Debug)]
pub struct StorableMockDaLayer {
    conn: DatabaseConnection,
    /// The height which is currently being built.
    next_height: u32,
    last_finalized_height: u32,
    /// Defines how many blocks should pass between receiving a blob and including it in a block.
    delay_blobs_by: u32,
    /// Defines how many blocks should be submitted before the block is finalized.
    /// Zero means instant finality.
    pub(crate) blocks_to_finality: u32,
    pub(crate) finalized_header_sender: broadcast::Sender<MockBlockHeader>,
    head_header_sender: watch::Sender<MockBlockHeader>,
    randomizer: Option<Randomizer>,
}

impl StorableMockDaLayer {
    /// Creates new [`StorableMockDaLayer`] by passing connections string directly to [`Database`]
    pub async fn new_from_connection(
        connection_string: &str,
        blocks_to_finality: u32,
    ) -> anyhow::Result<Self> {
        let mut opts = sea_orm::ConnectOptions::new(connection_string);

        opts.max_connections(50);
        opts.sqlx_logging_level(tracing::log::LevelFilter::Trace);

        let conn: DatabaseConnection = Database::connect(opts).await?;

        entity::setup_db(&conn).await?;
        let last_seen_block = entity::query_last_saved_block(&conn).await?;
        let next_height = (last_seen_block.height as u32)
            .checked_add(1)
            .expect("next_height overflow");

        let last_finalized_height = entity::query_last_finalized_height(&conn).await?;
        let (finalized_header_sender, mut rx) = broadcast::channel(100);

        // Spawn a task, so the receiver is not dropped, and the channel is not
        // closed.
        // Once the sender is dropped, the receiver will receive an
        // error and the task will exit.
        tokio::spawn(async move { while rx.recv().await.is_ok() {} });

        let (sender, _receiver) = watch::channel(last_seen_block);

        Ok(StorableMockDaLayer {
            conn,
            next_height,
            delay_blobs_by: 0,
            last_finalized_height,
            blocks_to_finality,
            finalized_header_sender,
            head_header_sender: sender,
            randomizer: None,
        })
    }

    /// Creates in-memory SQLite instance.
    pub async fn new_in_memory(blocks_to_finality: u32) -> anyhow::Result<Self> {
        Self::new_from_connection(&MockDaConfig::sqlite_in_memory(), blocks_to_finality).await
    }

    /// Creates an SQLite instance at a given path.
    pub async fn new_in_path(
        path: impl AsRef<std::path::Path>,
        blocks_to_finality: u32,
    ) -> anyhow::Result<Self> {
        let connection_string = MockDaConfig::sqlite_in_dir(path)?;
        Self::new_from_connection(&connection_string, blocks_to_finality).await
    }

    /// Produce new block with provided timestamp.
    pub async fn produce_block_with_timestamp(
        &mut self,
        timestamp: sov_rollup_interface::da::Time,
    ) -> anyhow::Result<()> {
        tracing::trace!(
            next_height = self.next_height,
            ?timestamp,
            "Start producing a new block at"
        );
        if self.next_height >= i32::MAX as u32 {
            anyhow::bail!("Due to database limitation cannot produce anymore blocks: {} is more than max supported height {}", self.next_height, i32::MAX);
        }

        let prev_block_hash = if self.next_height > 1 {
            let block = BlockHeaders::find()
                .filter(block_headers::Column::Height.eq(self.next_height - 1))
                .one(&self.conn)
                .await?
                .expect("Previous block is missing from the database");
            let hash: [u8; 32] = block.hash.try_into().map_err(|e: Vec<u8>| {
                anyhow::anyhow!(
                    "BlockHash should be 32 bytes long in database, but it is {}",
                    e.len()
                )
            })?;
            hash
        } else {
            GENESIS_HEADER.hash.0
        };

        let blobs = Blobs::find()
            .filter(blobs::Column::BlockHeight.eq(self.next_height + self.delay_blobs_by))
            .all(&self.conn)
            .await?;
        let blobs_count = blobs.len();
        tracing::trace!(
            blobs_count,
            height = self.next_height,
            "Extracted blobs for this block"
        );

        let this_block_hash = self.calculate_block_hash(self.next_height, &prev_block_hash, &blobs);

        let new_head = MockBlockHeader {
            height: self.next_height as u64,
            prev_hash: MockHash(prev_block_hash),
            hash: MockHash(this_block_hash),
            time: timestamp,
        };

        let block_model = block_headers::ActiveModel::from(new_head.clone());
        block_model.insert(&self.conn).await?;
        let _ = self.head_header_sender.send_replace(new_head);
        tracing::trace!(
            blobs_count,
            height = self.next_height,
            prev_hash = %HexHash::new(prev_block_hash),
            hash = %HexHash::new(this_block_hash),
            "New block has been produced"
        );

        self.next_height += 1;

        let next_finalized_height = self
            .next_height
            .checked_sub(self.blocks_to_finality.saturating_add(1))
            .unwrap_or_default();
        // Meaning that "chain head - blocks to finalization" has moved beyond genesis block.
        if next_finalized_height > 0 && next_finalized_height > self.last_finalized_height {
            self.last_finalized_height = next_finalized_height;
            finalized_height::update_value(&self.conn, self.last_finalized_height).await?;
            let finalized_header = self.get_header_at(next_finalized_height).await?;
            tracing::trace!(
                header = %finalized_header,
                "Submitting finalized header at"
            );
            match self.finalized_header_sender.send(finalized_header) {
                Ok(received_count) => {
                    tracing::trace!(receivers = received_count, "Finalized header sent");
                }
                Err(_) => {
                    tracing::info!(
                        "Failed to send finalized header notifications because no more listeners available."
                    );
                }
            };
        }
        Ok(())
    }

    /// Wait the specified number of blocks before including blobs on DA
    pub fn set_delay_blobs_by(&mut self, delay: u32) {
        self.delay_blobs_by = delay;
    }

    /// Saves new block header into a database.
    pub async fn produce_block(&mut self) -> anyhow::Result<()> {
        tracing::trace!(
            next_height = self.next_height,
            "Produce block has been called"
        );
        let timestamp = sov_rollup_interface::da::Time::now();

        // Temporarily remove the randomizer from `self` so it won't collide
        // with the &mut borrow needed in `produce_block`:
        let mut randomizer = self.randomizer.take();

        // Saving result and not using `?`, because need to restore randomizer back
        let start = std::time::Instant::now();
        let result = match &mut randomizer {
            None => self.produce_block_with_timestamp(timestamp).await,
            Some(randomizer) => randomizer.produce_block(self, timestamp).await,
        };
        self.randomizer = randomizer;
        tracing::trace!(
            ?result,
            time = ?start.elapsed(),
            "Produce block has been completed"
        );
        result
    }

    async fn get_header_at(&self, height: u32) -> anyhow::Result<MockBlockHeader> {
        if height < 1 {
            return Ok(GENESIS_HEADER);
        }
        if height >= self.next_height {
            anyhow::bail!("Block at height {} has not been produced yet", height);
        }
        let header = BlockHeaders::find()
            .filter(block_headers::Column::Height.eq(height))
            .one(&self.conn)
            .await?
            .map(MockBlockHeader::from)
            .expect("Corrupted DB, block not found");
        Ok(header)
    }

    pub(crate) async fn submit_batch(
        &mut self,
        batch_data: &[u8],
        sender: &MockAddress,
    ) -> anyhow::Result<MockHash> {
        tracing::trace!(
            batch_bytes = batch_data.len(),
            %sender,
            next_da_height = self.next_height,
            "Submitting batch is received"
        );
        let (blob, hash) = blobs::build_batch_blob(self.next_height as i32, batch_data, sender);
        blob.insert(&self.conn).await?;
        let include_at = self.next_height + self.delay_blobs_by;
        tracing::trace!(
            %hash,
            %sender,
            next_da_height = self.next_height,
            include_at = %include_at,
            "Submitted batch is saved"
        );
        Ok(hash)
    }

    pub(crate) async fn submit_proof(
        &mut self,
        proof_data: &[u8],
        sender: &MockAddress,
    ) -> anyhow::Result<MockHash> {
        tracing::trace!(
            proof_bytes = proof_data.len(),
            %sender,
            next_da_height = self.next_height,
            "Submitting proof is received"
        );
        let (blob, hash) = blobs::build_proof_blob(self.next_height as i32, proof_data, sender);
        blob.insert(&self.conn).await?;
        tracing::trace!(
            %hash,
            %sender,
            next_da_height = self.next_height,
            "Submitted proof is saved"
        );
        Ok(hash)
    }

    /// Get head block header saved in the database.
    pub async fn get_head_block_header(&self) -> anyhow::Result<MockBlockHeader> {
        self.get_header_at(self.next_height.saturating_sub(1)).await
    }

    /// Get updates on the latest head block
    pub fn subscribe_to_head_updates(&self) -> watch::Receiver<MockBlockHeader> {
        self.head_header_sender.subscribe()
    }

    pub(crate) async fn get_last_finalized_block_header(&self) -> anyhow::Result<MockBlockHeader> {
        self.get_header_at(self.last_finalized_height).await
    }

    pub(crate) async fn get_block_header_at(&self, height: u32) -> anyhow::Result<MockBlockHeader> {
        if height >= self.next_height {
            anyhow::bail!("Block at height {} has not been produced yet", height);
        }
        if height == 0 {
            return Ok(GENESIS_HEADER);
        }
        self.get_header_at(height).await
    }

    pub(crate) async fn get_block_at(&self, height: u32) -> anyhow::Result<MockBlock> {
        if height >= self.next_height {
            anyhow::bail!("Block at height {} has not been produced yet", height);
        }
        if height == 0 {
            return Ok(GENESIS_BLOCK);
        }

        let header = self.get_header_at(height).await?;

        let mut blobs = Blobs::find()
            .filter(blobs::Column::BlockHeight.eq(height))
            .all(&self.conn)
            .await?;

        // Batches are submitted more often,
        // so we are willing to pay for extra allocation when only proofs were submitted.
        let mut batch_blobs = Vec::with_capacity(blobs.len());
        let mut proof_blobs = Vec::new();

        if let Some(randomizer) = &self.randomizer {
            if randomizer.behaviour == RandomizationBehaviour::OutOfOrderBlobs {
                let mut hasher = sha2::Sha256::new();
                hasher.update(randomizer.rng.get_seed());
                // Adding height to have different order for different batches.
                hasher.update(height.to_le_bytes());
                // Adding hash to have different ordering for different forks.
                hasher.update(header.hash.0);
                let result = hasher.finalize();
                let mut hashed_seed = [0u8; 32];
                hashed_seed.copy_from_slice(&result[..32]);

                let mut rng = SmallRng::from_seed(hashed_seed);
                blobs.shuffle(&mut rng);
            }
        }

        for blob in blobs {
            match blob.namespace.as_str() {
                entity::BATCH_NAMESPACE => batch_blobs.push(MockBlob::from(blob)),
                entity::PROOF_NAMESPACE => proof_blobs.push(MockBlob::from(blob)),
                namespace => {
                    panic!("Unknown namespace: {namespace}, corrupted block")
                }
            }
        }

        Ok(MockBlock {
            header,
            batch_blobs,
            proof_blobs,
        })
    }

    /// Enables [`Randomizer`] for all new blocks. Alters behaviour of [`Self::produce_block`].
    pub fn set_randomizer(&mut self, randomizer: Randomizer) {
        self.randomizer = Some(randomizer);
    }

    /// Disables randomizer and returns an existing one.
    pub fn disable_randomizer(&mut self) -> Option<Randomizer> {
        self.randomizer.take()
    }

    /// Passed `height` becomes new head height.
    /// All previously submitted blobs above passed height are removed
    /// Newly submitted blobs will be included in `height + 1`.
    /// Returns an error if passed height below finalized height.
    pub async fn rewind_to_height(&mut self, height: u32) -> anyhow::Result<()> {
        let last_finalized_height = self.last_finalized_height;
        if height < last_finalized_height {
            anyhow::bail!(
                "Cannot rewind to height: {} because it is below last finalized height: {}",
                height,
                last_finalized_height
            );
        }

        Blobs::delete_many()
            .filter(blobs::Column::BlockHeight.gt(height))
            .exec(&self.conn)
            .await?;

        BlockHeaders::delete_many()
            .filter(block_headers::Column::Height.gt(height))
            .exec(&self.conn)
            .await?;

        let past_next_height = self.next_height;
        self.next_height = height + 1;
        tracing::info!(
            past_next_height,
            next_height = self.next_height,
            "StorableMockDaLayer rewound"
        );

        self.reload_head().await
    }

    async fn shuffle_non_finalized_blobs_inner<R: Rng>(
        &mut self,
        rng: &mut R,
        drop_blobs_percentage: u8,
        block_placeholder_upper_bound: Option<u32>,
    ) -> anyhow::Result<()> {
        let last_finalized_height = self.last_finalized_height;
        tracing::debug!(
            drop_blobs_percentage,
            last_finalized_height,
            ?block_placeholder_upper_bound,
            "Reshuffling non-finalized blocks"
        );

        let start_reading = std::time::Instant::now();
        // Query 1: Read a lot: all blobs data.
        let non_finalized_blobs = Blobs::find()
            .filter(blobs::Column::BlockHeight.gt(last_finalized_height))
            .all(&self.conn)
            .await?;
        tracing::trace!(
            non_finalized_blobs = non_finalized_blobs.len(),
            "Fetched non-finalized blobs"
        );

        // Query 2: Reads medium: block headers only
        let non_finalized_block_headers = BlockHeaders::find()
            .filter(block_headers::Column::Height.gt(last_finalized_height))
            .order_by_asc(block_headers::Column::Height)
            .all(&self.conn)
            .await?;

        // QUERY 3: Small, single query.
        // If performance is an issue, this can be hacked around and merged with querying other blocks.
        // Keep it simple now.
        let last_finalized_header = self.get_last_finalized_block_header().await?;
        tracing::trace!(time = ?start_reading.elapsed(), "Reading non finalized blocks and blobs completed");
        tracing::debug!(
            time = ?start_reading.elapsed(),
            blobs = non_finalized_blobs.len(),
            block_headers = non_finalized_block_headers.len(),
            "Reading data is completed");

        let updating_start = std::time::Instant::now();
        // This is going to be layout of new non-finalized blocks.
        let mut new_non_finalised_order: Vec<Vec<blobs::Model>> = (last_finalized_height
            ..self.next_height)
            .map(|_height| Vec::new())
            .collect();

        let max_relative_place = match block_placeholder_upper_bound {
            None => new_non_finalised_order.len(),
            Some(upper_bound) => std::cmp::min(upper_bound as usize, new_non_finalised_order.len()),
        };

        let mut blobs_to_drop = Vec::new();
        for blob in non_finalized_blobs {
            let choice = rng.gen_range(0..100);
            if choice < drop_blobs_percentage {
                tracing::trace!(?blob, ?choice, drop_blobs_percentage, "blob is dropped");
                blobs_to_drop.push(blob.id);
                continue;
            }
            // Note: Currently, it is possible that all blobs can be moved to non produced(next) block.
            // It is fine, just to keep in mind.
            let new_relative_height = rng.gen_range(0..max_relative_place);
            new_non_finalised_order[new_relative_height].push(blob);
        }

        let mut prev_hash = last_finalized_header.hash.0;

        // Query 4. Also impacts block_height index.
        Blobs::delete_many()
            .filter(blobs::Column::Id.is_in(blobs_to_drop))
            .exec(&self.conn)
            .await?;

        // 2*N update queries, minimum of data is written, but the blob index is rebuilt.
        for (block_header, blobs) in non_finalized_block_headers
            .into_iter()
            .zip(new_non_finalised_order)
        {
            let new_hash =
                self.calculate_block_hash(block_header.height as u32, &prev_hash, &blobs);
            let blobs_ids = blobs.iter().map(|blob| blob.id).collect::<Vec<_>>();
            let blobs_count = blobs_ids.len();

            Blobs::update_many()
                .filter(blobs::Column::Id.is_in(blobs_ids))
                .col_expr(
                    blobs::Column::BlockHeight,
                    sea_orm::prelude::Expr::value(block_header.height),
                )
                .exec(&self.conn)
                .await?;

            tracing::trace!(
                height = block_header.height,
                old_prev_hash = %(HexString::from(&block_header.prev_hash)),
                old_hash = %(HexString::from(&block_header.hash)),
                new_prev_hash = %HexHash::new(prev_hash),
                new_hash = %HexHash::new(new_hash),
                new_blobs_count = blobs_count,
                "Updating block header",
            );
            BlockHeaders::update_many()
                .filter(block_headers::Column::Height.eq(block_header.height))
                .col_expr(
                    block_headers::Column::Hash,
                    sea_orm::prelude::Expr::value(new_hash.to_vec()),
                )
                .col_expr(
                    block_headers::Column::PrevHash,
                    sea_orm::prelude::Expr::value(prev_hash.to_vec()),
                )
                .exec(&self.conn)
                .await?;
            prev_hash = new_hash;
        }
        tracing::trace!(time = ?updating_start.elapsed(), "Updating non finalized blocks completed");
        Ok(())
    }

    /// Shuffles blobs across non-finalized blocks to simulate real-world reorganization (reorg) scenarios.
    ///
    /// Non-finalized blocks are defined by Self.finality
    /// This method modifies the database state and can be safely repeated.
    ///
    /// Blobs are shuffled deterministically based on the provided random number generator (`rng`).
    /// Additionally, a percentage of blobs can be permanently dropped, as controlled by `drop_blobs_percentage`:
    /// - If `drop_blobs_percentage` is set to `0`, no blobs are dropped.
    /// - Dropped blobs are removed from the system entirely.
    ///
    /// # Errors
    /// - The method will return an error if communication with the underlying database fails.
    pub async fn shuffle_non_finalized_blobs<R: Rng>(
        &mut self,
        rng: &mut R,
        drop_blobs_percentage: u8,
    ) -> anyhow::Result<()> {
        self.shuffle_non_finalized_blobs_inner(rng, drop_blobs_percentage, None)
            .await?;
        self.reload_head().await
    }

    async fn reload_head(&self) -> anyhow::Result<()> {
        let new_head = query_last_saved_block(&self.conn).await?;
        self.head_header_sender.send_replace(new_head);
        Ok(())
    }

    fn calculate_block_hash(
        &self,
        height: u32,
        prev_block_hash: &[u8; 32],
        blobs: &[blobs::Model],
    ) -> [u8; 32] {
        let mut hasher = sha2::Sha256::new();

        hasher.update(height.to_be_bytes());
        hasher.update(prev_block_hash);

        for blob in blobs {
            hasher.update(&blob.hash[..]);
            hasher.update(&blob.sender[..]);
            hasher.update(&blob.namespace[..]);
        }

        hasher.finalize().into()
    }
}

/// Controller of the randomization behaviour for [`StorableMockDaLayer`].
/// Holds seed and behaviour.
#[derive(Clone, Debug)]
pub struct Randomizer {
    rng: rand_chacha::ChaChaRng,
    behaviour: RandomizationBehaviour,
    last_reorg_height: u32,
    reorg_interval: Range<u32>,
}

impl Randomizer {
    #[allow(missing_docs)]
    pub fn from_config(config: RandomizationConfig) -> Self {
        let rng = rand_chacha::ChaChaRng::from_seed(config.seed.0);
        Self {
            rng,
            behaviour: config.behaviour,
            last_reorg_height: 0,
            reorg_interval: config.reorg_interval,
        }
    }

    // Take `da_layer` because producing a new block can happen before or after a new block header is created.
    // So it is up to randomizer to decide.
    // Note: it should not call `StorableMockDaLayer::produce_block` because it will lead to infinite recursion.
    async fn produce_block(
        &mut self,
        da_layer: &mut StorableMockDaLayer,
        timestamp: sov_rollup_interface::da::Time,
    ) -> anyhow::Result<()> {
        let choice = self.rng.gen_range(self.reorg_interval.clone());
        let distance_from_last_reorg = da_layer.next_height.saturating_sub(self.last_reorg_height);
        let prev_reorg_height = self.last_reorg_height;
        let should_randomize = if distance_from_last_reorg >= choice {
            self.last_reorg_height = da_layer.next_height;
            true
        } else {
            false
        };
        tracing::trace!(
            should_randomize,
            choice,
            distance_from_last_reorg,
            next_height = da_layer.next_height,
            last_finalized_height = da_layer.last_finalized_height,
            prev_reorg_height = prev_reorg_height,
            last_reorg_height = self.last_reorg_height,
            "Will randomization be enabled on this call"
        );
        if should_randomize {
            match &self.behaviour {
                // This happens only on `get_block_at`, so we produce normal block all the time.
                RandomizationBehaviour::OutOfOrderBlobs => {
                    da_layer.produce_block_with_timestamp(timestamp).await?;
                }
                // Not supported currently
                RandomizationBehaviour::Rewind => {
                    // Produce the block first, otherwise data will be always lost.
                    da_layer.produce_block_with_timestamp(timestamp).await?;
                    let range = da_layer.last_finalized_height..da_layer.next_height;
                    let height_to_rewind = self.rng.gen_range(range);
                    da_layer.rewind_to_height(height_to_rewind).await?;
                }
                RandomizationBehaviour::ShuffleAndResize {
                    drop_percent,
                    adjust_head_height,
                } => {
                    let chosen_head_adjustment = self.rng.gen_range(adjust_head_height.clone());
                    match chosen_head_adjustment {
                        // below zero - rewind
                        ..0 => {
                            let minimal_possible_rewind = da_layer
                                .last_finalized_height
                                .checked_add(1)
                                .expect("end of chain");

                            let suggested_rewind = da_layer
                                .next_height
                                .saturating_sub(chosen_head_adjustment.unsigned_abs());

                            let height_to_rewind =
                                std::cmp::max(suggested_rewind, minimal_possible_rewind);

                            tracing::trace!(
                                chosen_head_adjustment,
                                adjust_head_height_range = ?adjust_head_height,
                                next_height = da_layer.next_height,
                                last_finalized_height = da_layer.last_finalized_height,
                                suggested_rewind,
                                "Choosing to rewinding to height"
                            );
                            let upper_bound =
                                height_to_rewind.saturating_sub(da_layer.last_finalized_height);
                            da_layer
                                .shuffle_non_finalized_blobs_inner(
                                    &mut self.rng,
                                    *drop_percent,
                                    Some(upper_bound),
                                )
                                .await?;
                            da_layer.produce_block_with_timestamp(timestamp).await?;
                            da_layer.rewind_to_height(height_to_rewind).await?;
                        }
                        // Just shuffle
                        0 => {
                            da_layer
                                .shuffle_non_finalized_blobs_inner(
                                    &mut self.rng,
                                    *drop_percent,
                                    None,
                                )
                                .await?;
                            da_layer.produce_block_with_timestamp(timestamp).await?;
                            da_layer.reload_head().await?;
                        }
                        // extend, if possible
                        1.. => {
                            let next_finalized_height = da_layer
                                .last_finalized_height
                                .checked_add(da_layer.blocks_to_finality)
                                .expect("end of chain");
                            let max_extending = next_finalized_height
                                .saturating_sub(da_layer.next_height)
                                .saturating_sub(1);
                            let extending =
                                std::cmp::min(chosen_head_adjustment as u32, max_extending);
                            tracing::trace!(
                                max_extending,
                                chosen_head_adjustment,
                                extending,
                                "Extending the chain by"
                            );

                            let last_finalized_height_before = da_layer.last_finalized_height;
                            for _i in 0..extending {
                                da_layer
                                    .produce_block_with_timestamp(timestamp.clone())
                                    .await?;
                            }
                            da_layer
                                .shuffle_non_finalized_blobs_inner(
                                    &mut self.rng,
                                    *drop_percent,
                                    None,
                                )
                                .await?;
                            // If not doing extending, just producing new block after shuffle is completed.
                            // But why? To advance chain. Produce block should always advance the chain.
                            if extending == 0 {
                                da_layer
                                    .produce_block_with_timestamp(timestamp.clone())
                                    .await?;
                            } else {
                                assert_eq!(
                                    last_finalized_height_before,
                                    da_layer.last_finalized_height
                                );
                            }
                            da_layer.reload_head().await?;
                        }
                    };
                }
            }
        } else {
            da_layer.produce_block_with_timestamp(timestamp).await?;
        }

        Ok(())
    }

    /// Allows producing [`Randomizer`] instance with new a behaviour,
    /// but retaining state of underlying `rng`.
    pub fn with_different_behaviour(self, behaviour: RandomizationBehaviour) -> Self {
        Self {
            rng: self.rng,
            behaviour,
            last_reorg_height: self.last_reorg_height,
            reorg_interval: self.reorg_interval,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap};
    use std::process::Command;
    use std::time::Duration;

    use proptest::prelude::*;
    use sov_rollup_interface::common::HexHash;
    use sov_rollup_interface::da::{BlobReaderTrait, BlockHeaderTrait};
    use sov_rollup_interface::node::da::SlotData;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use tokio::task::JoinHandle;
    use tokio::time;

    use super::*;
    use crate::MockAddress;

    enum TestBlob {
        Batch(Vec<u8>),
        Proof(Vec<u8>),
    }

    const DEFAULT_SENDER: MockAddress = MockAddress::new([1; 32]);
    const ISSUE_REMINDER: &str = "Leave a comment in https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1396 if you see this error";
    const ASYNC_OPERATION_TIMEOUT: Duration = Duration::from_secs(60);

    async fn check_da_layer_consistency(da_layer: &StorableMockDaLayer) -> anyhow::Result<()> {
        let mut prev_block_hash = GENESIS_HEADER.prev_hash;

        for height in 0..da_layer.next_height {
            let block = da_layer.get_block_at(height).await?;
            assert_eq!(height, block.header().height as u32);
            assert_eq!(
                prev_block_hash,
                block.header().prev_hash,
                "Prev block hash mismatch for block: {}",
                block.header(),
            );
            prev_block_hash = block.header().hash;
        }

        let last_finalized_header = da_layer.get_last_finalized_block_header().await?;
        let non_finalized_blocks = da_layer
            .next_height
            .saturating_sub(last_finalized_header.height as u32)
            .saturating_sub(1); // Allow to decrement "currently built block"
        assert!(
            non_finalized_blocks <= da_layer.blocks_to_finality,
            "Too many non finalized blocks={} when finality={}",
            non_finalized_blocks,
            da_layer.blocks_to_finality
        );

        Ok(())
    }

    async fn check_expected_blobs(
        da_layer: &StorableMockDaLayer,
        expected_blocks: &[Vec<(TestBlob, MockAddress)>],
    ) -> anyhow::Result<()> {
        // The current height is expected to be the next of the number of blocks sent.
        // Meaning da layer is "building next block".
        assert_eq!(expected_blocks.len() as u32 + 1, da_layer.next_height);
        check_da_layer_consistency(da_layer).await?;
        for (idx, expected_block) in expected_blocks.iter().enumerate() {
            let height = (idx + 1) as u32;
            let received_block = da_layer.get_block_at(height).await?;
            assert_eq!(height as u64, received_block.header().height);
            let mut batches = received_block.batch_blobs.into_iter();
            let mut proofs = received_block.proof_blobs.into_iter();

            for (blob, sender) in expected_block {
                let (mut received_blob, submitted_data) = match blob {
                    TestBlob::Batch(submitted_batch) => {
                        let received_batch =
                            batches.next().expect("Missed batch data in received block");
                        (received_batch, submitted_batch)
                    }
                    TestBlob::Proof(submitted_proof) => {
                        let received_proof =
                            proofs.next().expect("Missed proof data in received block");
                        (received_proof, submitted_proof)
                    }
                };

                assert_eq!(
                    sender, &received_blob.address,
                    "Sender mismatch in received blob"
                );
                assert_eq!(&submitted_data[..], received_blob.full_data());
            }

            // No extra more batches were in received block
            assert!(batches.next().is_none());
            assert!(proofs.next().is_none());
        }
        Ok(())
    }

    fn get_finalized_headers_collector(
        da: &StorableMockDaLayer,
        expected_num_headers: usize,
    ) -> JoinHandle<Vec<MockBlockHeader>> {
        let mut receiver = da.finalized_header_sender.subscribe();
        tokio::spawn(async move {
            let mut received = Vec::with_capacity(expected_num_headers);
            for i in 0..expected_num_headers {
                match time::timeout(ASYNC_OPERATION_TIMEOUT, receiver.recv()).await {
                    Ok(Ok(header)) => received.push(header),
                    Err(time) => {
                        panic!(
                            "Timeout waiting for finalized header {}, {:?}. {}",
                            i + 1,
                            time,
                            ISSUE_REMINDER
                        )
                    }
                    Ok(Err(err)) => panic!(
                        "Finalized header channel has been closed at height {}: {:?}. {}",
                        i + 1,
                        err,
                        ISSUE_REMINDER,
                    ),
                }
            }
            received
        })
    }

    // Gets vector of blocks.
    // Block contains Vec of blobs and sender.
    // Checks that submission works and finalized headers are sent.
    // And that same data is there after reopening the same file again.
    async fn submit_blobs_and_restart(
        connection_string: &str,
        blocks: Vec<Vec<(TestBlob, MockAddress)>>,
    ) -> anyhow::Result<()> {
        // Iteration 1, submit and check.
        {
            let mut da_layer =
                StorableMockDaLayer::new_from_connection(connection_string, 0).await?;
            let finalized_headers_collector =
                get_finalized_headers_collector(&da_layer, blocks.len());
            let mut prev_head_block_header = GENESIS_HEADER;
            for block in &blocks {
                for (blob, sender) in block {
                    match blob {
                        TestBlob::Batch(batch) => {
                            da_layer.submit_batch(batch, sender).await?;
                        }
                        TestBlob::Proof(proof) => {
                            da_layer.submit_proof(proof, sender).await?;
                        }
                    }
                }
                da_layer.produce_block().await?;
                let head_block_header = da_layer.get_head_block_header().await?;
                assert_eq!(
                    prev_head_block_header.height() + 1,
                    head_block_header.height()
                );
                assert_eq!(prev_head_block_header.hash(), head_block_header.prev_hash());
                prev_head_block_header = head_block_header;
            }
            check_expected_blobs(&da_layer, &blocks).await?;
            let finalized_headers = finalized_headers_collector.await?;
            assert_eq!(
                blocks.len(),
                finalized_headers.len(),
                "Incorrect number of finalized headers received",
            );
            let mut prev_block_hash = GENESIS_HEADER.hash;
            for (idx, header) in finalized_headers.iter().enumerate() {
                assert_eq!(idx as u64 + 1, header.height());
                assert_eq!(prev_block_hash, header.prev_hash());
                prev_block_hash = header.hash;
            }
        }

        // Iteration 2, load from disk and check.
        {
            // Open from disk again.
            let da_layer = StorableMockDaLayer::new_from_connection(connection_string, 0).await?;
            check_expected_blobs(&da_layer, &blocks).await?;
        }

        Ok(())
    }

    fn check_block_batch(block: &mut MockBlock, idx: usize, expected: &[u8]) {
        let batch = block.batch_blobs.get_mut(idx).unwrap();
        assert_eq!(expected, batch.full_data());
    }

    fn check_block_proof(block: &mut MockBlock, idx: usize, expected: &[u8]) {
        let proof = block.proof_blobs.get_mut(idx).unwrap();
        assert_eq!(expected, proof.full_data());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_layer() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;

        let da_layer = StorableMockDaLayer::new_in_path(tempdir.path(), 0).await?;

        let head_block_header = da_layer.get_head_block_header().await?;
        assert_eq!(GENESIS_HEADER, head_block_header);
        let head_block = da_layer.get_block_at(GENESIS_HEADER.height as u32).await?;
        assert_eq!(GENESIS_BLOCK, head_block);
        let last_finalized_height = da_layer.last_finalized_height;
        assert_eq!(0, last_finalized_height);

        // Non-existing
        let response = da_layer.get_block_at(1).await;
        assert!(response.is_err());
        assert_eq!(
            "Block at height 1 has not been produced yet",
            response.unwrap_err().to_string()
        );

        let response = da_layer.get_header_at(1).await;
        assert!(response.is_err());
        assert_eq!(
            "Block at height 1 has not been produced yet",
            response.unwrap_err().to_string()
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn submit_batches_and_restart_regular_sqlite() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let db_path = tempdir.path().join("mock_da.sqlite");
        let connection_string = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());

        let sender_1 = MockAddress::new([1; 32]);
        let sender_2 = MockAddress::new([2; 32]);

        // Blobs per each block, with sender
        let expected_blocks = vec![
            // Block 1
            vec![
                (TestBlob::Batch(vec![1, 1, 1, 1]), sender_1),
                (TestBlob::Batch(vec![1, 1, 2, 2]), sender_2),
            ],
            // Block 2
            vec![
                (TestBlob::Batch(vec![2, 2, 1, 1]), sender_1),
                (TestBlob::Batch(vec![2, 2, 2, 2]), sender_2),
                (TestBlob::Batch(vec![2, 2, 3, 3]), sender_1),
            ],
        ];

        submit_blobs_and_restart(&connection_string, expected_blocks).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn submit_batches_and_restart_with_empty_blocks() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let db_path = tempdir.path().join("mock_da.sqlite");
        let connection_string = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());

        let expected_blocks = vec![
            // Block 1
            vec![(TestBlob::Batch(vec![1, 1, 1, 1]), DEFAULT_SENDER)],
            // Block 2
            Vec::new(),
            // Block 3,
            Vec::new(),
            // Block 4
            vec![
                (TestBlob::Batch(vec![4, 4, 1, 1]), DEFAULT_SENDER),
                (TestBlob::Batch(vec![4, 4, 3, 3]), DEFAULT_SENDER),
            ],
        ];

        submit_blobs_and_restart(&connection_string, expected_blocks).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn submit_batches_and_proofs_and_restart_regular() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let db_path = tempdir.path().join("mock_da.sqlite");
        let connection_string = format!("sqlite://{}?mode=rwc", db_path.to_string_lossy());
        let sender_1 = MockAddress::new([1; 32]);
        let sender_2 = MockAddress::new([2; 32]);
        let sender_3 = MockAddress::new([3; 32]);

        // Blobs per each block, with sender
        let expected_blocks = vec![
            // Block 1
            vec![
                (TestBlob::Batch(vec![1, 1, 1, 1]), sender_1),
                (TestBlob::Proof(vec![1, 1, 2, 2]), sender_2),
                (TestBlob::Batch(vec![1, 1, 3, 3]), sender_2),
                (TestBlob::Batch(vec![1, 1, 4, 4]), sender_3),
                (TestBlob::Proof(vec![1, 1, 5, 5]), sender_1),
            ],
            // Block 2
            vec![
                (TestBlob::Batch(vec![2, 2, 1, 1]), sender_1),
                (TestBlob::Proof(vec![2, 2, 2, 2]), sender_2),
                (TestBlob::Batch(vec![2, 2, 3, 3]), sender_2),
                (TestBlob::Proof(vec![2, 2, 4, 4]), sender_3),
                (TestBlob::Batch(vec![2, 2, 5, 5]), sender_1),
            ],
        ];

        submit_blobs_and_restart(&connection_string, expected_blocks).await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn close_before_producing_block() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;

        let batch_1 = vec![1, 1, 1, 1];
        let batch_2 = vec![1, 1, 2, 2];
        let batch_3 = vec![1, 1, 3, 3];
        let batch_4 = vec![1, 1, 4, 4];
        let proof_1 = vec![2, 2, 1, 1];
        let proof_2 = vec![2, 2, 2, 2];
        let proof_3 = vec![2, 2, 3, 3];
        let proof_4 = vec![2, 2, 4, 4];

        {
            let mut da_layer = StorableMockDaLayer::new_in_path(tempdir.path(), 0).await?;
            check_da_layer_consistency(&da_layer).await?;
            da_layer.submit_batch(&batch_1, &DEFAULT_SENDER).await?;
            da_layer.submit_proof(&proof_1, &DEFAULT_SENDER).await?;
        }
        {
            let mut da_layer = StorableMockDaLayer::new_in_path(tempdir.path(), 0).await?;
            da_layer.submit_batch(&batch_2, &DEFAULT_SENDER).await?;
            da_layer.submit_proof(&proof_2, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
            check_da_layer_consistency(&da_layer).await?;
        }
        {
            let mut da_layer = StorableMockDaLayer::new_in_path(tempdir.path(), 0).await?;
            check_da_layer_consistency(&da_layer).await?;
            da_layer.submit_batch(&batch_3, &DEFAULT_SENDER).await?;
            da_layer.submit_proof(&proof_3, &DEFAULT_SENDER).await?;
        }
        {
            let mut da_layer = StorableMockDaLayer::new_in_path(tempdir.path(), 0).await?;
            da_layer.submit_batch(&batch_4, &DEFAULT_SENDER).await?;
            da_layer.submit_proof(&proof_4, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
            check_da_layer_consistency(&da_layer).await?;
        }
        // Checking
        {
            let da_layer = StorableMockDaLayer::new_in_path(tempdir.path(), 0).await?;
            check_da_layer_consistency(&da_layer).await?;
            let head_block_header = da_layer.get_head_block_header().await?;
            assert_eq!(2, head_block_header.height());
            let mut block_1 = da_layer.get_block_at(1).await?;
            assert_eq!(2, block_1.batch_blobs.len());
            assert_eq!(2, block_1.proof_blobs.len());
            check_block_batch(&mut block_1, 0, &batch_1[..]);
            check_block_batch(&mut block_1, 1, &batch_2[..]);
            check_block_proof(&mut block_1, 0, &proof_1[..]);
            check_block_proof(&mut block_1, 1, &proof_2[..]);

            let mut block_2 = da_layer.get_block_at(2).await?;
            check_block_batch(&mut block_2, 0, &batch_3[..]);
            check_block_batch(&mut block_2, 1, &batch_4[..]);
            check_block_proof(&mut block_2, 0, &proof_3[..]);
            check_block_proof(&mut block_2, 1, &proof_4[..]);
        }

        Ok(())
    }

    fn is_docker_running() -> bool {
        Command::new("docker")
            .arg("version")
            .output()
            .is_ok_and(|output| output.status.success())
    }

    #[tokio::test(flavor = "multi_thread")]
    #[cfg_attr(not(feature = "postgres"), ignore)]
    async fn test_postgresql_existing() -> anyhow::Result<()> {
        if !is_docker_running() {
            eprintln!("Docker is not running, skipping test.");
            return Ok(());
        }

        let node = Postgres::default().start().await?;

        // prepare connection string
        let connection_string = &format!(
            "postgres://postgres:postgres@127.0.0.1:{}/postgres",
            node.get_host_port_ipv4(5432).await?
        );

        let sender_1 = DEFAULT_SENDER;
        let sender_2 = MockAddress::new([2; 32]);

        // Blobs per each block, with sender
        let expected_blocks = vec![
            // Block 1
            vec![
                (TestBlob::Batch(vec![1, 1, 1, 1]), sender_1),
                (TestBlob::Batch(vec![1, 1, 2, 2]), sender_2),
            ],
            // Block 2
            vec![
                (TestBlob::Batch(vec![2, 2, 1, 1]), sender_1),
                (TestBlob::Batch(vec![2, 2, 2, 2]), sender_2),
                (TestBlob::Batch(vec![2, 2, 3, 3]), sender_1),
            ],
        ];

        submit_blobs_and_restart(connection_string, expected_blocks).await
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore]
    async fn generate_mock_da_with_many_empty_blocks() -> anyhow::Result<()> {
        let test_data = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test_data")
            .join("10k_empty_blocks.sqlite");
        let connection_string = format!("sqlite://{}?mode=rwc", test_data.to_string_lossy());
        let mut layer = StorableMockDaLayer::new_from_connection(&connection_string, 0).await?;
        for _ in 0..10_000 {
            layer.produce_block().await?;
        }
        Ok(())
    }

    /// The idea of the test is
    /// checking that [`StorableMockDaLayer`] can return blobs out of order if a related option is set.
    #[tokio::test(flavor = "multi_thread")]
    async fn blobs_out_of_order_works_for_single_block() -> anyhow::Result<()> {
        // Initialize some batch and proofs and submit them in a single block
        let blobs = vec![
            (TestBlob::Batch(vec![10, 10]), DEFAULT_SENDER),
            (TestBlob::Batch(vec![2, 2]), DEFAULT_SENDER),
            (TestBlob::Proof(vec![30, 30]), DEFAULT_SENDER),
            (TestBlob::Batch(vec![4, 4]), DEFAULT_SENDER),
            (TestBlob::Proof(vec![5, 5]), DEFAULT_SENDER),
            (TestBlob::Proof(vec![60, 60]), DEFAULT_SENDER),
            (TestBlob::Batch(vec![7, 7]), DEFAULT_SENDER),
        ];

        let mut in_order_batches = Vec::new();
        let mut in_order_proofs = Vec::new();
        let mut da_layer = StorableMockDaLayer::new_in_memory(1).await?;
        for (blob, sender) in &blobs {
            match blob {
                TestBlob::Batch(batch) => {
                    da_layer.submit_batch(batch, sender).await?;
                    in_order_batches.push(batch.clone());
                }
                TestBlob::Proof(proof) => {
                    da_layer.submit_proof(proof, sender).await?;
                    in_order_proofs.push(proof.clone());
                }
            }
        }
        da_layer.produce_block().await?;

        // First, we validate that blobs are returned in the same way they were submitted.
        let in_order_block = da_layer.get_block_at(1).await?;
        let (actual_in_order_batches, actual_in_order_proofs) = get_raw_data(in_order_block);

        assert_eq!(actual_in_order_batches, in_order_batches);
        assert_eq!(actual_in_order_proofs, in_order_proofs);

        // Now let's change the ordering.
        let randomizer = Randomizer::from_config(RandomizationConfig {
            seed: HexHash::new([42; 32]),
            reorg_interval: 1..da_layer.blocks_to_finality,
            behaviour: RandomizationBehaviour::OutOfOrderBlobs,
        });
        da_layer.set_randomizer(randomizer);

        let out_of_order_block = da_layer.get_block_at(1).await?;
        let (mut out_of_order_batches, mut out_of_order_proofs) = get_raw_data(out_of_order_block);

        // They are not equal, because unordered.
        assert_ne!(out_of_order_batches, in_order_batches);
        assert_ne!(out_of_order_proofs, in_order_proofs);

        // But if we sort them, they are equal, meaning that data is the same!
        out_of_order_batches.sort();
        out_of_order_proofs.sort();
        let mut sorted_submitted_batches = in_order_batches.clone();
        sorted_submitted_batches.sort();
        let mut sorted_submitted_proofs = in_order_proofs.clone();
        sorted_submitted_proofs.sort();

        assert_eq!(out_of_order_batches, sorted_submitted_batches);
        assert_eq!(out_of_order_proofs, sorted_submitted_proofs);

        // Disabling randomization retrieval makes brings submission order back
        let _ = da_layer.disable_randomizer();
        let in_order_block = da_layer.get_block_at(1).await?;
        let (batches_after_disabling, proofs_after_disabling) = get_raw_data(in_order_block);

        assert_eq!(batches_after_disabling, in_order_batches);
        assert_eq!(proofs_after_disabling, in_order_proofs);
        Ok(())
    }

    /// To make sure that randomization is different between blocks.
    /// Test validates that by submitting the same blobs in the same order in different blocks
    /// Then checking that order is different.
    #[tokio::test(flavor = "multi_thread")]
    async fn blobs_out_of_order_different_order_for_different_blocks() -> anyhow::Result<()> {
        // We only test batches for simplicity.
        let batches = vec![
            vec![1, 1],
            vec![4, 4],
            vec![3, 3],
            vec![8, 8],
            vec![9, 9],
            vec![11, 11],
        ];
        let blocks = 5;
        let mut da_layer = StorableMockDaLayer::new_in_memory(1).await?;
        for _ in 0..blocks {
            for batch in &batches {
                da_layer.submit_batch(batch, &DEFAULT_SENDER).await?;
            }
            da_layer.produce_block().await?;
        }

        let randomizer = Randomizer::from_config(RandomizationConfig {
            seed: HexHash::new([42; 32]),
            reorg_interval: 1..da_layer.blocks_to_finality,
            behaviour: RandomizationBehaviour::OutOfOrderBlobs,
        });
        da_layer.set_randomizer(randomizer);
        // Batches fetched in each block
        let mut seen_batches_per_block: Vec<Vec<Vec<u8>>> = Vec::new();
        for height in 1..=blocks {
            let block = da_layer.get_block_at(height).await?;
            let (fetched_batches_this_block, _) = get_raw_data(block);
            for previous_batches in &seen_batches_per_block {
                assert_ne!(&fetched_batches_this_block, previous_batches);
            }
            seen_batches_per_block.push(fetched_batches_this_block);
        }

        Ok(())
    }

    // Returns tuple of all raw batches and raw proofs
    fn get_raw_data(mut block: MockBlock) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
        let mut batches = Vec::new();
        let mut proofs = Vec::new();

        for batch in block.batch_blobs.iter_mut() {
            let batch_data = batch.full_data().to_vec();
            batches.push(batch_data);
        }
        for proof in block.proof_blobs.iter_mut() {
            let proof_data = proof.full_data().to_vec();
            proofs.push(proof_data);
        }
        (batches, proofs)
    }

    /// This test ensures that old blobs are removed after rewinding and that only new blobs
    /// are returned in their place. Specifically, it validates the behavior of the data
    /// availability (DA) layer's rewinding mechanism by simulating a fork scenario.
    ///
    /// Test steps:
    /// 1. Set finalization to 10 blocks.
    /// 2. Submit 15 batches (1 per block). At this point, block 5 becomes the last finalized height.
    /// 3. Rewind to height 5, effectively starting a new fork from that point.
    /// 4. Submit 10 more blocks, making the head height return to 15.
    /// 5. Fetch all blobs in the chain and validate:
    ///     - Up to the rewind point (height 5), the original blobs remain.
    ///     - Beyond the rewind point, the blobs from the fork are included.
    #[tokio::test(flavor = "multi_thread")]
    async fn storable_mock_da_rewinds_and_replaces_blobs() -> anyhow::Result<()> {
        let finality = 10;
        let end_height = 15;
        // The height **after** which new fork is going to be built.
        let fork_height = 5;
        let mut da_layer = StorableMockDaLayer::new_in_memory(finality).await?;
        let head_block_receiver = da_layer.subscribe_to_head_updates();

        // Submit the first 15 blobs
        let original_blobs: Vec<Vec<u8>> = (1u8..=end_height).map(|x| vec![x, x]).collect();
        for blob in &original_blobs {
            da_layer.submit_batch(blob, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
            let head_block_received = head_block_receiver.borrow().clone();
            let head_block = da_layer.get_head_block_header().await?;
            assert_eq!(head_block_received, head_block);
        }

        let head_header_before = da_layer.get_head_block_header().await?;
        assert_eq!(head_header_before.height(), end_height as u64);
        let last_finalized_header_before = da_layer.get_last_finalized_block_header().await?;
        assert_eq!(last_finalized_header_before.height(), fork_height as u64);

        da_layer.rewind_to_height(fork_height).await?;

        let head_header_after = da_layer.get_head_block_header().await?;
        assert_eq!(head_header_after.height(), fork_height as u64);
        let head_received = head_block_receiver.borrow().clone();
        assert_eq!(head_received, head_header_after);

        let last_finalized_header_after = da_layer.get_last_finalized_block_header().await?;
        // No change in the last finalized header.
        assert_eq!(last_finalized_header_after, last_finalized_header_before);

        // Submit another 10 blobs
        let fork_blobs: Vec<Vec<u8>> = ((fork_height as u8 + 1)..=end_height)
            .map(|x| vec![x * 10, x])
            .collect();
        for blob in &fork_blobs {
            da_layer.submit_batch(blob, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
        }

        // Iterate through the chain from the beginning and validate blobs
        let mut all_fetched_blobs = Vec::new();
        for height in 1..=15 {
            let block = da_layer.get_block_at(height).await?;
            let (fetched_batches, _) = get_raw_data(block);
            all_fetched_blobs.extend(fetched_batches);
        }

        // Original blobs up to fork point plus new blobs after.
        let expected_blobs: Vec<_> = (1u8..=(fork_height as u8))
            .map(|x| vec![x, x])
            .chain(((fork_height as u8 + 1)..=end_height).map(|x| vec![x * 10, x]))
            .collect();

        assert_eq!(all_fetched_blobs, expected_blobs);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cannot_rewind_below_finalized_height() -> anyhow::Result<()> {
        let finality = 3;
        let mut da_layer = StorableMockDaLayer::new_in_memory(finality).await?;
        da_layer.rewind_to_height(0).await?;
        for _ in 0..=finality {
            da_layer.produce_block().await?;
        }
        let err = da_layer.rewind_to_height(0).await.unwrap_err();
        assert_eq!(
            err.to_string(),
            "Cannot rewind to height: 0 because it is below last finalized height: 1"
        );

        da_layer.produce_block().await?;
        let result = da_layer.rewind_to_height(1).await;
        assert!(result.is_err());

        Ok(())
    }

    /// We want to make sure, that when rewind happens,
    /// last_finalized height will be shown correctly even if rewinding has happened.
    #[tokio::test(flavor = "multi_thread")]
    async fn test_last_finalized_height_saved_between_restarts() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let finality = 3;
        let blocks = 5;
        let expected_last_finalized_height = 2;
        // Create blocks, so finalization happens.
        // Rewind to the last finalized height.
        {
            let mut da_layer = StorableMockDaLayer::new_in_path(tempdir.path(), finality).await?;
            for _ in 0..blocks {
                da_layer.produce_block().await?;
            }
            assert_eq!(
                da_layer.get_last_finalized_block_header().await?.height(),
                expected_last_finalized_height
            );
            assert_eq!(
                da_layer.get_head_block_header().await?.height(),
                blocks as u64
            );
            da_layer
                .rewind_to_height(expected_last_finalized_height as u32)
                .await?;
        }
        // Last finalized height == head
        {
            let mut da_layer = StorableMockDaLayer::new_in_path(tempdir.path(), finality).await?;
            assert_eq!(
                da_layer.get_last_finalized_block_header().await?.height(),
                expected_last_finalized_height
            );
            assert_eq!(
                da_layer.get_head_block_header().await?.height(),
                expected_last_finalized_height,
            );
            // Producing new blocks ensures finalize height increased correctly.
            for i in 1..=finality {
                da_layer.produce_block().await?;
                assert_eq!(
                    da_layer.get_last_finalized_block_header().await?.height(),
                    expected_last_finalized_height
                );
                assert_eq!(
                    da_layer.get_head_block_header().await?.height(),
                    expected_last_finalized_height + i as u64,
                );
            }
            // Now last finalized head increases
            da_layer.produce_block().await?;
            assert_eq!(
                da_layer.get_last_finalized_block_header().await?.height(),
                expected_last_finalized_height + 1
            );
            assert_eq!(
                da_layer.get_head_block_header().await?.height(),
                expected_last_finalized_height + finality as u64 + 1,
            );
        }
        Ok(())
    }

    /// The number of blocks to finalization is a parameter to the constructor.
    /// Thus, it is possible to change it for the same database.
    /// Imagine this scenario.
    /// 1. DaLayer initialized with finalization of 5 blocks.
    /// 2. 10 blocks are produced, so the last finalized height is 5.
    /// 3. DaLayer is closed and re-opened with the finalization of 3 blocks.
    /// 4. 11th block is produced. The last finalized height is 11 - 3 = 8.
    ///
    /// By design, StorableMockDaLayer only send notification about the last finalized height.
    ///
    #[tokio::test(flavor = "multi_thread")]
    async fn finalization_notifications_are_skipped_when_parameter_changes() -> anyhow::Result<()> {
        let tempdir = tempfile::tempdir()?;
        let initial_finality = 5;
        let changed_finality = 3;
        let first_round_blocks = 10;
        let expected_initial_finalized_height = 5;
        let mut finalized_heights_received = Vec::new();
        {
            let mut da_layer =
                StorableMockDaLayer::new_in_path(tempdir.path(), initial_finality).await?;
            let mut rx = da_layer.finalized_header_sender.subscribe();

            for _ in 0..first_round_blocks {
                da_layer.produce_block().await?;
                if let Ok(finalized_header) = rx.try_recv() {
                    finalized_heights_received.push(finalized_header.height());
                }
            }
            assert_eq!(
                da_layer.get_last_finalized_block_header().await?.height(),
                expected_initial_finalized_height
            );
            assert_eq!(
                da_layer.get_head_block_header().await?.height(),
                first_round_blocks as u64
            );
        }
        {
            let mut da_layer =
                StorableMockDaLayer::new_in_path(tempdir.path(), changed_finality).await?;
            let mut rx = da_layer.finalized_header_sender.subscribe();

            da_layer.produce_block().await?;
            let finalized_header = rx.try_recv()?;
            finalized_heights_received.push(finalized_header.height());
        }
        // We acknowledge the skipping of finalized heights 6 and 7, because parameters have changed.
        // Technically, this can be fixed in the future.
        let expected_finalized_heights = vec![1, 2, 3, 4, 5, 8];
        assert_eq!(finalized_heights_received, expected_finalized_heights);
        Ok(())
    }

    /// This test performs shuffling and validation of the result.
    /// Some of the validation relies on random behaviour,
    /// so it can only be reliably checked with high enough numbers.
    /// Thus
    async fn reshuffling_test(
        seed: [u8; 32],
        finality_blocks: u32,
        blocks_to_process: u8,
        drop_percentage: u8,
        blob_size: usize,
    ) -> anyhow::Result<()> {
        let mut rng = SmallRng::from_seed(seed);
        let mut da_layer = StorableMockDaLayer::new_in_memory(finality_blocks).await?;
        let head_block_receiver = da_layer.subscribe_to_head_updates();

        // We submit 1 batch per block, with batch content derived from height.
        for height in 1..=blocks_to_process {
            let blob = vec![height; blob_size];
            da_layer.submit_batch(&blob, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
            let head_block_received = head_block_receiver.borrow().clone();
            let head_block = da_layer.get_head_block_header().await?;
            assert_eq!(head_block_received, head_block);
        }

        let head_before = da_layer.get_head_block_header().await?;
        let finalized_before = da_layer.get_last_finalized_block_header().await?;

        da_layer
            .shuffle_non_finalized_blobs(&mut rng, drop_percentage)
            .await?;

        check_da_layer_consistency(&da_layer).await?;
        let head_after = da_layer.get_head_block_header().await?;
        let head_received = head_block_receiver.borrow().clone();
        assert_eq!(head_received, head_after);
        let finalized_after = da_layer.get_last_finalized_block_header().await?;
        // Head block can only change with finality is
        // finality == 0: no shuffling happens
        // finality == 1: 1 out of 1 blob is shuffled -> no visible effect.
        // finality == 2: 50% probability that a blob will remain in the same block, making no change.
        if finality_blocks > 3 {
            assert_ne!(head_before, head_after);
        }
        assert_eq!(head_before.height(), head_after.height());
        // Finalized unchanged
        assert_eq!(finalized_before, finalized_after);
        let last_finalized_height = finalized_after.height();

        // Verify that blobs have crossed block boundaries.
        // Blob content derived deterministically from height, so we can spot "foreign blob"
        let mut has_alien_blob = false;
        let mut non_finalized_batches_fetch_count = 0;
        for height in 1..=blocks_to_process {
            let expected_batch = vec![height; blob_size];
            let mut block = da_layer.get_block_at(height as u32).await?;
            if height as u64 <= last_finalized_height {
                // Finalized data shouldn't be changed.
                assert_eq!(
                    block.batch_blobs.len(),
                    1,
                    "data has been added to finalized block"
                );
                let mut blob = block.batch_blobs.pop().unwrap();
                let data = blob.full_data().to_vec();
                assert_eq!(data, expected_batch, "finalized block got shuffled");
            } else {
                non_finalized_batches_fetch_count += block.batch_blobs.len();
                for batch in block.batch_blobs.iter_mut() {
                    let batch_data = batch.full_data().to_vec();
                    if batch_data != expected_batch {
                        has_alien_blob = true;
                    }
                }
            }
        }

        // To observe the effects of shuffling, make sure there are enough blocks to shuffle.
        // This condition ensures that at least 10 non-finalized blocks have been submitted
        // and are eligible for shuffling.
        if finality_blocks > 10 && blocks_to_process > 20 {
            match drop_percentage {
                0 => {
                    // We are certain that no blocks are dropped.
                    // The number of fetched non-finalized batches must match the total finality blocks.
                    assert!(has_alien_blob);
                    // It is either cut of by finality, or not reached finality at all.
                    let expected_non_finalized_fetch_count =
                        std::cmp::min(finality_blocks as usize, blocks_to_process as usize);
                    assert_eq!(
                        non_finalized_batches_fetch_count, expected_non_finalized_fetch_count,
                        "something got dropped when it shouldn't!"
                    );
                }
                1..=49 => {
                    // A shuffle is expected; however, due to a relatively low drop percentage,
                    // we do not assert that anything was necessarily dropped.
                    assert!(has_alien_blob);
                }
                50..=99 => {
                    // We expect some blocks to be dropped.
                    // We do not assert that a shuffle must occur because a single blob
                    // can remain in the same position purely by chance.
                    assert!(
                        non_finalized_batches_fetch_count < finality_blocks as usize,
                        "nothing got dropped"
                    );
                }
                100..=u8::MAX => {
                    // Since everything is set to be dropped, no blocks should be fetched.
                    assert_eq!(0, non_finalized_batches_fetch_count);
                    // No blobs remain, so we cannot detect any effect of shuffling.
                    // If this assertion fails, there is a bug in the test.
                    assert!(!has_alien_blob, "something was shuffled!");
                }
            }
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shuffling_standard() -> anyhow::Result<()> {
        // Drop nothing
        reshuffling_test([12; 32], 40, 100, 0, 50_000).await?;
        // Drop all
        reshuffling_test([12; 32], 40, 100, 100, 50_000).await?;
        reshuffling_test([12; 32], 38, 21, 0, 50_000).await?;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reorg_interval_works() -> anyhow::Result<()> {
        let finality = 10;
        // Exactly after each 5 blocks
        let reorg_interval = 5..6;

        let mut da_layer = StorableMockDaLayer::new_in_memory(finality).await?;
        da_layer.set_randomizer(Randomizer::from_config(RandomizationConfig {
            seed: HexHash::new([1; 32]),
            reorg_interval: reorg_interval.clone(),
            behaviour: RandomizationBehaviour::only_shuffle(0),
        }));
        let blob = [10; 10];
        let mut seen_blocks: Vec<MockBlockHeader> = Vec::new();

        let when_fork = reorg_interval.start;

        for height in 1..=27 {
            da_layer.submit_batch(&blob, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
            let block_header = da_layer.get_header_at(height).await?;
            assert_eq!(block_header.height(), height as u64);
            tracing::debug!(?block_header, height, "block");
            check_da_layer_consistency(&da_layer).await?;
            if height == 1 {
                tracing::info!("first block, skipping");
                continue;
            } else if height % when_fork == 0 {
                tracing::info!(height, "EXPECTING A REORG");
                let last = seen_blocks.last().expect("There should be the last");
                assert_ne!(
                    last.hash(),
                    block_header.prev_hash(),
                    "reorg didn't happen when it should"
                );
                let mut new_chain = Vec::with_capacity(height as usize);
                for h in 1..=height {
                    let block_header = da_layer.get_header_at(h).await?;
                    new_chain.push(block_header.clone());
                }
                seen_blocks = new_chain;
            } else {
                tracing::info!(height, "NO REORG");
                seen_blocks.push(block_header.clone());
                // In every other case no reorg have happened
                for i in 1..seen_blocks.len() {
                    let prev_block = &seen_blocks[i - 1];
                    let current_block = &seen_blocks[i];
                    assert_eq!(
                        current_block.prev_hash(),
                        prev_block.hash(),
                        "Reorg happened, when it shouldn't .Block at height {} is not pointing to the correct previous block. {:?}",
                        i + 1,
                        seen_blocks,
                    );
                }
            }
        }

        Ok(())
    }

    // Check that when the upper bound is provided,
    // no blobs are going to be in the blocks above this bound
    #[tokio::test(flavor = "multi_thread")]
    async fn test_shuffle_upper_bound() -> anyhow::Result<()> {
        let finality = 10;
        let blocks_to_process = 18;
        let mut rng = SmallRng::from_seed([1; 32]);
        let mut da_layer = StorableMockDaLayer::new_in_memory(finality).await?;

        let mut sent_blobs = Vec::with_capacity(blocks_to_process);

        for idx in 0..blocks_to_process {
            let blob: Vec<u8> = vec![idx as u8; 10];
            da_layer.submit_batch(&blob, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
            sent_blobs.push(blob);
        }

        // height 10 is finalized, so height 11, 12, 13, 14 are expected to have all blobs now
        let upper_bound = 5;

        da_layer
            .shuffle_non_finalized_blobs_inner(&mut rng, 0, Some(upper_bound))
            .await?;

        let mut after_shuffle_blobs = Vec::with_capacity(blocks_to_process);

        let height_without_blobs = finality + upper_bound + 1;

        for height in 1..=blocks_to_process {
            let block = da_layer.get_block_at(height as u32).await?;
            if height > height_without_blobs as usize {
                assert!(
                    block.batch_blobs.is_empty(),
                    "blobs should not be placed in blocks above the upper bound={upper_bound} height={height}"
                );
            } else {
                for mut blob in block.batch_blobs.into_iter() {
                    let data = blob.full_data().to_vec();
                    after_shuffle_blobs.push(data);
                }
            }
        }

        assert_ne!(sent_blobs, after_shuffle_blobs, "blobs should be shuffled");
        sent_blobs.sort();
        after_shuffle_blobs.sort();

        assert_eq!(sent_blobs, after_shuffle_blobs);

        Ok(())
    }

    // Check that the new fork height changes in both directions!
    #[tokio::test(flavor = "multi_thread")]
    async fn test_rewind_and_extend() -> anyhow::Result<()> {
        // std::env::set_var("RUST_LOG", "debug,sov_mock_da=trace");
        // sov_test_utils::initialize_logging();
        let finality = 30;
        let target_height = 60;
        let fork_depth = 2..4;
        let mut da_layer = StorableMockDaLayer::new_in_memory(finality).await?;
        let head_block_receiver = da_layer.subscribe_to_head_updates();
        let seed = HexHash::new([120; 32]);
        let adjust_head_range = -20..10;
        da_layer.set_randomizer(Randomizer::from_config(RandomizationConfig {
            seed,
            reorg_interval: fork_depth.clone(),
            behaviour: RandomizationBehaviour::ShuffleAndResize {
                drop_percent: 10,
                adjust_head_height: adjust_head_range.clone(),
            },
        }));

        let mut height = 1;
        let mut iterations: u64 = 0;
        let mut was_rewound = false;
        let mut was_extended = false;

        let mut head = da_layer.get_head_block_header().await?;

        let mut seen_heights: HashMap<u64, u64> = HashMap::new();
        let mut seen_reorg_heights: BTreeMap<u64, u64> = BTreeMap::new();

        while height <= target_height {
            iterations += 1;
            let blob = iterations.to_be_bytes().to_vec();
            da_layer.submit_batch(&blob, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
            check_da_layer_consistency(&da_layer).await?;

            *seen_heights.entry(height as u64).or_insert(0) += 1;

            let current_head = da_layer.get_head_block_header().await?;
            let head_block_received = head_block_receiver.borrow().clone();
            assert_eq!(head_block_received, current_head);

            let head_diff = current_head.height() as i32 - head.height() as i32;
            if current_head.prev_hash() != head.hash() {
                *seen_reorg_heights.entry(head.height()).or_insert(0) += 1;
            }
            if head_diff < 0 {
                was_rewound = true;
            } else if head_diff > 1 {
                was_extended = true;
            }

            head = current_head;
            height = (head.height() + 1) as u32;
        }

        assert!(was_rewound, "Rewinding didn't happen");
        assert!(was_extended, "Extending didn't happen");

        let seen_more_than_once = seen_heights.values().filter(|v| **v > 1).count();
        let seen_more_than_2_times = seen_heights.values().filter(|v| **v > 2).count();
        assert!(
            seen_more_than_once > 0,
            "Never seen on the same height twice"
        );
        assert!(
            seen_more_than_2_times > 0,
            "Never seen on the same height more than twice"
        );
        // TODO: Investigate later how to make reorg behaviour to land on the previously seen heights
        let _landed_more_than_once = seen_reorg_heights.values().filter(|v| **v > 1).count();
        let _landed_more_than_2_times = seen_reorg_heights.values().filter(|v| **v > 2).count();
        // assert!(
        //     landed_more_than_once > 0,
        //     "Never landed on the same height twice"
        // );
        // assert!(
        //     landed_more_than_2_times > 0,
        //     "Never landed on the same height more than twice"
        // );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_shuffle_and_rewind_no_drop() -> anyhow::Result<()> {
        let finality = 10;
        let fork_depth = 5..6;
        let mut da_layer = StorableMockDaLayer::new_in_memory(finality).await?;
        da_layer.set_randomizer(Randomizer::from_config(RandomizationConfig {
            seed: HexHash::new([120; 32]),
            reorg_interval: fork_depth.clone(),
            behaviour: RandomizationBehaviour::ShuffleAndResize {
                drop_percent: 0,
                adjust_head_height: -3..-1,
            },
        }));

        let when_fork = fork_depth.start;
        let mut height = 1;
        let mut iterations: u64 = 0;
        let mut head = da_layer.get_head_block_header().await?;
        let mut seen_blocks = Vec::new();

        let mut was_rewound = false;
        let mut was_shuffled = true;
        while height <= 17 {
            iterations += 1;
            let blob = iterations.to_be_bytes().to_vec();
            da_layer.submit_batch(&blob, &DEFAULT_SENDER).await?;
            da_layer.produce_block().await?;
            check_da_layer_consistency(&da_layer).await?;
            if height % when_fork == 0 {
                let current_head = da_layer.get_head_block_header().await?;
                // Rewinding happened
                if current_head.height() < head.height() {
                    was_rewound = true;
                }
                head = current_head;
                let mut new_seen = Vec::with_capacity(height as usize);
                // Validating shuffling
                for h in 1..=head.height() {
                    let block_header = da_layer.get_header_at(h as u32).await?;
                    // Only check for previous blocks, current was not put in seen yet
                    if h < head.height() && !seen_blocks.contains(&block_header) {
                        was_shuffled = true;
                    }
                    new_seen.push(block_header);
                }
                seen_blocks = new_seen;
                height = (head.height() + 1) as u32;
            } else {
                tracing::info!(height, iterations, "no fork");
                let current_head = da_layer.get_head_block_header().await?;
                // Just checking head for simplicity.
                assert_eq!(head.hash(), current_head.prev_hash());
                head = current_head;
                let block_header = da_layer.get_header_at(height).await?;
                // But track seen headers for validating shuffle in the other branch
                seen_blocks.push(block_header);
                height += 1;
            }
        }

        assert!(was_rewound, "Rewinding didn't happen even once!");
        assert!(was_shuffled, "Shuffle didn't happen even once!");

        let mut seen_blobs = Vec::new();
        for h in 1..height {
            let block = da_layer.get_block_at(h).await?;
            for mut blob in block.batch_blobs.into_iter() {
                let data = blob.full_data().to_vec();
                seen_blobs.push(data);
            }
        }
        assert_eq!(
            seen_blobs.len(),
            iterations as usize,
            "Seen blobs don't match iterations. If left is less, some blobs were dropped"
        );
        seen_blobs.sort();
        let expected_blobs = (1..=iterations)
            .map(|i| i.to_be_bytes().to_vec())
            .collect::<Vec<_>>();

        assert_eq!(seen_blobs, expected_blobs, "Expected blobs mismatch");

        Ok(())
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(10))]
        #[test]
        fn prop_reshuffling_test(
            blocks_to_finality in 0u32..100u32,
            drop_percentage in 0u8..100u8,
            num_blocks in 10u8..250u8,
            seed in prop::array::uniform32(any::<u8>()),
        ) {
            let fut = async move {
                reshuffling_test(seed, blocks_to_finality, num_blocks, drop_percentage, 1_000).await
            };

            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async {
                    tokio::time::timeout(ASYNC_OPERATION_TIMEOUT, fut)
                        .await
                        .expect("Test timed out")
                        .expect("Test failed");
                });
        }
    }
}
