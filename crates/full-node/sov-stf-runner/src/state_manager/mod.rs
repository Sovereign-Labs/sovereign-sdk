//! All code related to handling storage manager anb ledger.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::da::DaServiceWithCachedFinalizedHeaders;
use crate::processes::{Sender as StfInfoSender, StateTransitionInfo};
use crate::query_state_update_info;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_db::ledger_db::{LedgerDb, SlotCommit};
use sov_db::schema::{DeltaReader, SchemaBatch};
use sov_metrics::RunnerProcessStfChangesMetrics;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::node::da::{DaService, SlotData};
use sov_rollup_interface::node::DaSyncState;
use sov_rollup_interface::stf::TxReceiptContents;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::StateTransitionWitness;
use sov_rollup_interface::{ProvableHeightTracker, StateUpdateInfo};
use tokio::sync::watch;

/// Result of checking if a block is a valid continuation of the current chain.
///
/// Used by runner to determine whether to process the block or fetch a different one.
pub enum BlockCandidateResolution<StfPreState, StateRoot, LedgerPreState> {
    /// Block is a valid continuation of a previously seen transition.
    /// Runner should proceed with STF execution using the provided pre-state.
    KnownContinuation {
        /// Valid pre-state for the requested block.
        /// STF can safely rely on this state for execution.
        pre_state: StfPreState,
        /// The state root before processing this block. matches `pre_state`
        pre_state_root: StateRoot,
        ledger_pre_state: LedgerPreState,
    },
    /// Block is not a valid continuation (reorg detected or caught up).
    /// Runner should fetch block at `height_to_fetch` and try again.
    NoMatch {
        /// The DA height runner should fetch next.
        /// This is the first unprocessed height in the current fork.
        height_to_fetch: u64,
    },
}

/// Structure that holds a block header and a pre-state root that was on this block header
struct StateOnBlock<Da: DaSpec, StateRoot> {
    block_header: Da::BlockHeader,
    pre_state_root: StateRoot,
    post_state_root: StateRoot,
}

impl<Da: DaSpec, StateRoot: AsRef<[u8]>> std::fmt::Debug for StateOnBlock<Da, StateRoot> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StateOnBlock")
            .field("block_header", &self.block_header)
            .field("pre_state_root", &hex::encode(self.pre_state_root.as_ref()))
            .field(
                "post_state_root",
                &hex::encode(self.post_state_root.as_ref()),
            )
            .finish()
    }
}

impl<Da: DaSpec, StateRoot: Clone> StateOnBlock<Da, StateRoot> {
    fn from_transition_witness<W>(
        transition_witness: &StateTransitionWitness<StateRoot, W, Da>,
    ) -> Self {
        StateOnBlock {
            block_header: transition_witness.da_block_header.clone(),
            pre_state_root: transition_witness.initial_state_root.clone(),
            post_state_root: transition_witness.final_state_root.clone(),
        }
    }
}

enum ForkPointSearchResult<Da: DaService> {
    /// Found a valid fork point - the first unprocessed block whose predecessor was seen.
    Found {
        height: u64,
    },
    HeadChanged(<Da::Spec as DaSpec>::BlockHeader),
    DaHeadIsBelowProcessedFinalized,
    /// All seen blocks are in current chain up to DA head.
    /// Caller should fetch and process block at the given height.
    NeedFutureBlock {
        height: u64,
    },
}

/// Outcome of binary search for fork point.
enum BinarySearchOutcome<Da: DaService> {
    /// Found a definitive result (Found or HeadChanged).
    Done(ForkPointSearchResult<Da>),
    /// Search exhausted without finding fork point - needs further handling.
    Exhausted {
        low: u64,
        high: u64,
        final_candidate: <Da::Spec as DaSpec>::BlockHeader,
    },
}

/// StateManager controls storage lifecycle for [`StateTransitionFunction`],
/// [`LedgerDb`] and API endpoints in case of DA-reorgs.
/// It needs [`DaService`] so it can backtrack to the last seen transition in new fork.
pub struct StateManager<StateRoot, Witness, Sm, Da>
where
    Da: DaService,
    Sm: HierarchicalStorageManager<Da::Spec>,
{
    storage_manager: Sm,
    ledger_db: LedgerDb,

    genesis_da_height: u64,
    /// The state root at the last processed finalized block.
    /// Used as fallback when `state_on_block` is empty (genesis/instant finality).
    last_processed_finalized_state_root: StateRoot,
    last_processed_finalized_header: <<Da as DaService>::Spec as DaSpec>::BlockHeader,
    // We record all seen transitions at the given height.
    state_on_block:
        HashMap<<<Da as DaService>::Spec as DaSpec>::SlotHash, StateOnBlock<Da::Spec, StateRoot>>,
    // Helper for faster iteration over fork tree.
    seen_on_height: BTreeMap<u64, HashSet<<Da::Spec as DaSpec>::SlotHash>>,
    state_update_sender: watch::Sender<StateUpdateInfo<Sm::StfState>>,
    stf_info_sender: Option<StfInfoSender<StateRoot, Witness, Da::Spec>>,
    max_provable_slot_number_tracker: Box<dyn ProvableHeightTracker>,
    is_initialized: bool,
    da_sync_state: Arc<DaSyncState>,
    finalized_headers_provider: DaServiceWithCachedFinalizedHeaders<Da>,
}

impl<StateRoot, Witness, Sm, Da> StateManager<StateRoot, Witness, Sm, Da>
where
    Da: DaService<Error = anyhow::Error>,
    StateRoot: Clone + AsRef<[u8]> + Serialize + DeserializeOwned,
    Witness: Serialize + DeserializeOwned,
    Sm: HierarchicalStorageManager<
        Da::Spec,
        LedgerChangeSet = SchemaBatch,
        LedgerState = DeltaReader,
    >,
    Sm::StfState: Clone,
{
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        storage_manager: Sm,
        ledger_db: LedgerDb,
        last_processed_finalized_state_root: StateRoot,
        state_update_channel: watch::Sender<StateUpdateInfo<Sm::StfState>>,
        stf_info_sender: Option<StfInfoSender<StateRoot, Witness, Da::Spec>>,
        state_height_tracker: Box<dyn ProvableHeightTracker>,
        da_sync_state: Arc<DaSyncState>,
        finalized_headers_provider: DaServiceWithCachedFinalizedHeaders<Da>,
        genesis_da_height: u64,
        last_processed_finalized_header: <<Da as DaService>::Spec as DaSpec>::BlockHeader,
    ) -> Self {
        Self {
            storage_manager,
            ledger_db,
            last_processed_finalized_state_root,
            last_processed_finalized_header,
            state_on_block: Default::default(),
            seen_on_height: Default::default(),
            state_update_sender: state_update_channel,
            stf_info_sender,
            max_provable_slot_number_tracker: state_height_tracker,
            is_initialized: false,
            da_sync_state,
            finalized_headers_provider,
            genesis_da_height,
        }
    }

    pub(crate) async fn startup(&mut self) -> anyhow::Result<()> {
        if let Some(sender) = &mut self.stf_info_sender {
            // If this state manager uses a channel, it MUST be correctly
            // initialized before usage.
            // Get ledger head for reconciliation - use genesis height if empty
            let ledger_head = self
                .ledger_db
                .get_head_slot()?
                .map(|(slot, _)| slot)
                .unwrap_or(SlotNumber::GENESIS);

            sender
                .startup_notify_about_infos_from_db(
                    ledger_head,
                    &*self.max_provable_slot_number_tracker,
                )
                .await?;
        }
        self.is_initialized = true;
        Ok(())
    }

    /// Checks if a block is a valid continuation of the current chain state.
    ///
    /// Returns:
    /// - `KnownContinuation { pre_state, pre_state_root }` if the block can be processed
    /// - `NoMatch { height_to_fetch }` if runner should fetch a different block
    #[tracing::instrument(skip_all)]
    pub(crate) async fn check_continuation(
        &mut self,
        block_header: &<Da::Spec as DaSpec>::BlockHeader,
        da_service: &Da,
    ) -> anyhow::Result<BlockCandidateResolution<Sm::StfState, StateRoot, Sm::LedgerState>> {
        let start = std::time::Instant::now();
        if !self.is_initialized {
            anyhow::bail!(
                "StateManager wasn't initialized. Please call `.startup()` method before using"
            );
        }

        let is_continuation = self.is_continuation(block_header);
        tracing::trace!(is_continuation, "Checked if block is continuation");

        if !is_continuation {
            // Block is not a continuation - find the fork point and tell runner what to fetch
            let height_to_fetch = match self.choose_fork_point(da_service).await? {
                ForkPointSearchResult::Found { height } => {
                    tracing::info!(
                        original_block = %block_header.display(),
                        fork_point_height = height,
                        time = ?start.elapsed(),
                        "Found fork point, runner should fetch this block"
                    );
                    height
                }
                ForkPointSearchResult::NeedFutureBlock { height } => {
                    tracing::trace!(
                        height,
                        time = ?start.elapsed(),
                        "All seen blocks in current chain, returning next height to fetch"
                    );
                    height
                }
                ForkPointSearchResult::HeadChanged(new_head) => {
                    // DA reorged during fork point search.
                    // If chain rewound below requested height, retry at new head + 1.
                    // If chain extended or same level, retry original height.
                    let retry_height = if new_head.height() < block_header.height() {
                        new_head.height().saturating_add(1)
                    } else {
                        block_header.height()
                    };
                    tracing::warn!(
                        new_head = %new_head.display(),
                        original_height = block_header.height(),
                        retry_height,
                        time = ?start.elapsed(),
                        "DA reorged during fork point search, runner should retry"
                    );
                    retry_height
                }
                ForkPointSearchResult::DaHeadIsBelowProcessedFinalized => {
                    // DA is behind our finalized height. Tell runner to fetch from after finalized.
                    let retry_height = self
                        .last_processed_finalized_header
                        .height()
                        .checked_add(1)
                        .expect("height overflow");
                    tracing::warn!(
                        last_finalized_height = self.last_processed_finalized_header.height(),
                        retry_height,
                        time = ?start.elapsed(),
                        "DA head is below finalized height, runner should retry"
                    );
                    retry_height
                }
            };
            return Ok(BlockCandidateResolution::NoMatch { height_to_fetch });
        }

        // Block is a continuation - get pre_state_root and create state
        let pre_state_root = self.get_pre_state_root_for_contiuation(block_header);
        let (pre_state, ledger_state) = self.storage_manager.create_state_for(block_header)?;

        tracing::trace!(
            block_header = %block_header.display(),
            time = ?start.elapsed(),
            "Block is a continuation, returning STF state"
        );

        Ok(BlockCandidateResolution::KnownContinuation {
            pre_state,
            pre_state_root,
            ledger_pre_state: ledger_state,
        })
    }

    /// Returns the pre-state root for processing this block.
    ///
    /// PRECONDITION: `is_continuation(block_header)` must be true.
    /// Panics if called on a non-continuation block.
    fn get_pre_state_root_for_contiuation(
        &self,
        block_header: &<Da::Spec as DaSpec>::BlockHeader,
    ) -> StateRoot {
        // Case 1: predecessor is in state_on_block
        if let Some(predecessor) = self.state_on_block.get(&block_header.prev_hash()) {
            return predecessor.post_state_root.clone();
        }

        // Case 2: block is directly after finalized header
        assert_eq!(
            block_header.prev_hash(),
            self.last_processed_finalized_header.hash(),
            "get_pre_state_root_for called on non-continuation block: \
            prev_hash={} does not match finalized_hash={} and is not in state_on_block",
            block_header.prev_hash(),
            self.last_processed_finalized_header.hash()
        );

        self.last_processed_finalized_state_root.clone()
    }

    /// Performs all necessary operations on data that has been processed by the rollup.
    /// All necessary data for these finalized transitions have been saved on disk.
    /// Now: First ever call to this method should be with either finalized block or with the block next to finalized.
    /// Otherwise follow up calls to prepare storage can panic if a chain reorgs.
    pub(crate) async fn process_stf_changes<
        S: SlotData,
        B: serde::Serialize,
        T: TxReceiptContents,
    >(
        &mut self,
        stf_changes: Sm::StfChangeSet,
        ledger_pre_state: Sm::LedgerState,
        transition_witness: StateTransitionWitness<StateRoot, Witness, Da::Spec>,
        slot_commit: SlotCommit<S, B, T>,
        aggregated_proofs: Vec<SerializedAggregatedProof>,
    ) -> anyhow::Result<()> {
        let start = std::time::Instant::now();
        if !self.is_initialized {
            anyhow::bail!(
                "StateManager wasn't initialized. Please call `.startup()` method before using"
            );
        }
        let new_state_root = transition_witness.final_state_root.clone();
        let block_header: <<Da as DaService>::Spec as DaSpec>::BlockHeader =
            transition_witness.da_block_header.clone();
        if self.state_on_block.contains_key(&block_header.hash()) {
            anyhow::bail!(
                "Attempt to process already processed block: {}, probably a bug in a caller",
                block_header.display()
            );
        }

        let aggregated_proofs_count = aggregated_proofs.len();
        tracing::debug!(
            current_state_root = hex::encode(self.last_processed_finalized_state_root.as_ref()),
            next_state_root = hex::encode(new_state_root.as_ref()),
            aggregated_proofs = aggregated_proofs_count,
            "Saving changes after applying slot"
        );

        // ---
        let seen_state_on_block =
            StateOnBlock::from_transition_witness::<Witness>(&transition_witness);
        tracing::trace!(block_header = %block_header.display(), "Adding transition to the list of the seen");
        self.pre_validate_transition_witness(&transition_witness, &seen_state_on_block);

        self.state_on_block
            .insert(block_header.hash(), seen_state_on_block);
        self.seen_on_height
            .entry(block_header.height())
            .or_default()
            .insert(block_header.hash());
        // ----

        let processing_finalized_transitions_start = std::time::Instant::now();
        let finalized_transitions = self.process_finalized_state_transitions().await?;
        let processing_finalized_transitions_time =
            processing_finalized_transitions_start.elapsed();
        tracing::trace!(
            finalized_transitions = finalized_transitions.len(),
            time = ?processing_finalized_transitions_time,
            "Processed finalized transitions"
        );

        self.ledger_db.replace_reader(ledger_pre_state);
        self.verify_transition_witness_against_ledger_state(&transition_witness)?;

        let slot_number = self.get_slot_number()?;
        let ledger_materialization_start = std::time::Instant::now();
        let mut ledger_change_set = self
            .ledger_db
            .materialize_slot(slot_commit, new_state_root.as_ref())?;
        tracing::trace!(
            prev_slot_number = ?slot_number,
            "Initial Ledger ChangeSet is materialized"
        );

        if let Some(finalized_transition) = finalized_transitions.iter().last() {
            let last_processed_finalized_header = &finalized_transition.block_header;
            let last_finalized_slot_number = SlotNumber::new_dangerous(
                last_processed_finalized_header
                    .height()
                    .saturating_sub(self.genesis_da_height),
            );
            tracing::trace!(
                ?last_finalized_slot_number,
                "Going to materialize last finalized slot number"
            );
            let last_finalized_slot_update = self
                .ledger_db
                .materialize_latest_finalize_slot(slot_number, last_finalized_slot_number)?;

            ledger_change_set.merge(last_finalized_slot_update);
            tracing::trace!(
                ?last_finalized_slot_number,
                current_slot_number = ?slot_number,
                "Last finalized slot is materialized into LedgerDb ChangeSet"
            );
        }

        if let Some(stf_info_sender) = &self.stf_info_sender {
            tracing::trace!("Going to stage StateTransitionInfo in ProofManagerDb");
            let stf_info = StateTransitionInfo {
                data: transition_witness,
                slot_number,
            };
            // Stage STF info data before ledger commit. Metadata is updated only after
            // the ledger commit succeeds to avoid advancing ProofManager beyond LedgerDb.
            stf_info_sender.stage_stf_info(&stf_info).await?;
            tracing::trace!("StateTransitionInfo is staged in ProofManagerDb");
        }

        for aggregated_proof in aggregated_proofs {
            let this_height_data = self
                .ledger_db
                .materialize_aggregated_proof(slot_number, aggregated_proof)?;
            ledger_change_set.merge(this_height_data);
            tracing::trace!("Aggregated Proof is materialized into Ledger ChangeSet");
        }
        let ledger_materialization_time = ledger_materialization_start.elapsed();
        tracing::trace!(time = ?ledger_materialization_time, "Materialized all LegerDb changes");

        let save_start = std::time::Instant::now();
        self.storage_manager
            .save_change_set(&block_header, stf_changes, ledger_change_set)?;
        let save_time = save_start.elapsed();

        let updating_api_time = self.update_api_and_ledger_storage(&block_header).await?;

        let finalize_start = std::time::Instant::now();
        for finalized_transition in &finalized_transitions {
            self.storage_manager
                .finalize(&finalized_transition.block_header)?;
        }
        let commit_time = finalize_start.elapsed();
        tracing::trace!(
            ?save_time,
            ?commit_time,
            "All finalized transitions are marked as finalized"
        );

        let sending_to_prover_start = std::time::Instant::now();
        if let Some(stf_info_sender) = &mut self.stf_info_sender {
            // Commit STF info metadata only after the ledger commit succeeds.
            stf_info_sender.commit_stf_info(slot_number).await?;

            // Notify `StateTransitionInfo` consumers that the data is saved in the Db.
            let max_provable_slot_number = self
                .max_provable_slot_number_tracker
                .max_provable_slot_number();
            tracing::trace!(
                ?max_provable_slot_number,
                "Going to notify stf_info_sender about max provable slot"
            );
            stf_info_sender.notify(max_provable_slot_number).await?;
            tracing::trace!(
                ?max_provable_slot_number,
                "State transition info receiver has been notified about max provable slot number"
            );
        }
        let sending_to_prover_time = sending_to_prover_start.elapsed();

        sov_metrics::track_metrics(|tracker| {
            tracker.submit(RunnerProcessStfChangesMetrics {
                da_height: block_header.height(),
                aggregated_proofs_count,
                finalized_transitions_count: finalized_transitions.len(),
                total_time: start.elapsed(),
                processing_finalized_transitions_time,
                ledger_changes_materializing_time: ledger_materialization_time,
                saving_to_storage_time: save_time,
                committing_storage_time: commit_time,
                updating_api_storage_time: updating_api_time,
                sending_stf_info_time_to_prover_time: sending_to_prover_time,
            });
        });
        tracing::trace!(
            time = ?start.elapsed(),
            "StateManager has processed STF changes");

        Ok(())
    }

    /// Updates both the [`LedgerDb`] and the [`StateUpdateInfo`]
    /// states.
    ///
    /// ## Potential synchronization issues
    /// Note that we are not using strong synchronization primitives here.
    /// However, we always have the guarantee that the [`LedgerDb`] is
    /// updated before the [`StateUpdateInfo`]. This means that, given the rollup height
    /// accessible from the [`StateUpdateInfo`] channel, we can safely query data from the [`LedgerDb`] at this height.
    async fn update_channels(
        &mut self,
        stf_state: Sm::StfState,
        ledger_state: DeltaReader,
    ) -> anyhow::Result<()> {
        self.ledger_db.replace_reader(ledger_state);
        let state_update_info =
            query_state_update_info(&self.ledger_db, stf_state, self.da_sync_state.as_ref())
                .await?;

        // `send_replace` is superior to `send` for our use case. It never fails
        // because it doesn't need to notify all receivers, unlike `send`, which
        // we don't need. It will also keep working even if there are no
        // receivers currently alive, which makes it easier to reason about the
        // code.
        self.state_update_sender.send_replace(state_update_info);

        Ok(())
    }

    /// Returns true if the block is a valid continuation of some of the previously processed blocks.
    /// "current" chain is determined by runner, which increments height
    ///
    /// A block is a continuation if:
    /// - It has NOT been processed yet:
    ///      - not in `state_on_block`
    ///      - it is ABOVE the last processed finalized height (blocks at or below finalized height have already been processed)
    /// - Its predecessor HAS been processed (in `state_on_block`) OR its prev_hash matches the finalized header
    fn is_continuation(&self, block_header: &<Da::Spec as DaSpec>::BlockHeader) -> bool {
        // Block at or below finalized height has already been processed
        if block_header.height() <= self.last_processed_finalized_header.height() {
            tracing::trace!(
                block_height = block_header.height(),
                finalized_height = self.last_processed_finalized_header.height(),
                block_hash = %block_header.hash(),
                "Block at or below finalized height, not a continuation"
            );
            return false;
        }

        tracing::trace!(
            block_header = %block_header.display(),
            last_processed_finalized_header = %self.last_processed_finalized_header.display(),
            state_on_block_len = self.state_on_block.len(),
            "Checking if block is a continuation");

        let processed = self.state_on_block.contains_key(&block_header.hash());
        // NOTE: Probably can do early return to safe memory accesses, but readability wins here.

        // Block follows finalized header directly (genesis/instant finality/post-reorg case)
        let adjacent_to_last_finalized =
            block_header.prev_hash() == self.last_processed_finalized_header.hash();

        // Normal case: predecessor is in state_on_block
        let has_seen_predecessor = self.state_on_block.contains_key(&block_header.prev_hash());

        let is_continuation = !processed && (adjacent_to_last_finalized || has_seen_predecessor);
        tracing::trace!(
            block_header = %block_header.display(),
            processed,
            adjacent_to_last_finalized,
            has_seen_predecessor,
            is_continuation,
            "Continuation check result"
        );
        is_continuation
    }

    /// Returns true if the block header represents a valid fork point during binary search.
    ///
    /// A block is a valid fork point if:
    /// - It has NOT been processed yet (not in `state_on_block`) and above last processed finalized height.
    /// - Its predecessor HAS been processed (in `state_on_block`)
    ///
    /// Note: Unlike `is_continuation`, this does not check the finalized header case
    /// since fork point search only runs when `state_on_block` is non-empty.
    fn is_valid_fork_point(&self, block_header: &<Da::Spec as DaSpec>::BlockHeader) -> bool {
        let processed = self.state_on_block.contains_key(&block_header.hash());
        let has_seen_predecessor = self.state_on_block.contains_key(&block_header.prev_hash());
        !processed && has_seen_predecessor
    }

    fn get_earliest_seen_height(&self) -> Option<u64> {
        self.seen_on_height.first_key_value().map(|(k, _)| *k)
    }

    // The highest seen does not mean the latest in the current chain.
    fn get_highest_seen_height(&self) -> Option<u64> {
        self.seen_on_height.last_key_value().map(|(k, _)| *k)
    }

    fn get_prev_hash(
        &self,
        hash: &<Da::Spec as DaSpec>::SlotHash,
    ) -> <Da::Spec as DaSpec>::SlotHash {
        self.state_on_block
            .get(hash)
            .map(|state| state.block_header.prev_hash())
            .expect("Internal inconsistency in maps")
    }

    // If reorg happened,
    // the next incremental continuation of that fork that hasn't been processed should be found.
    // Returns a ForkPointSearchResult that caller should handle:
    // - Found: proceed with the fork point block
    // - HeadChanged: DA reorged during search, caller should retry
    // - DaHeadIsBelowProcessedFinalized: DA is behind, caller should wait and retry
    // - NeedFutureBlock: all seen blocks are in current chain, fetch at given height
    async fn choose_fork_point(
        &self,
        da_service: &Da,
    ) -> anyhow::Result<ForkPointSearchResult<Da>> {
        // If we haven't seen anything we can only start from last seen finalized height.
        if self.state_on_block.is_empty() {
            let adjacent_height = self
                .last_processed_finalized_header
                .height()
                .checked_add(1)
                .expect("Reached end of the DA");
            return Ok(ForkPointSearchResult::Found {
                height: adjacent_height,
            });
        }

        let highest_seen_height = self
            .get_highest_seen_height()
            .expect("Choosing fork point only possible if some transitions have been seen");

        let head = da_service.get_head_block_header().await?;

        // Single attempt - caller handles retries
        self.try_find_candidate_in_current_chain(da_service, head, highest_seen_height)
            .await
    }

    /// Tries to find a candidate in the current state of the chain.
    /// If it notices that the chain has changed, it returns head of the new chain.
    async fn try_find_candidate_in_current_chain(
        &self,
        da_service: &Da,
        head: <Da::Spec as DaSpec>::BlockHeader,
        highest_seen_height: u64,
    ) -> anyhow::Result<ForkPointSearchResult<Da>> {
        let last_processed_finalized_height = self.last_processed_finalized_header.height();
        if head.height() <= last_processed_finalized_height {
            tracing::info!(
                passed = head.height(),
                last_processed_finalized_height,
                "Passed height is below or equal of what has been processed"
            );
            return Ok(ForkPointSearchResult::DaHeadIsBelowProcessedFinalized);
        }

        let low = last_processed_finalized_height
            .checked_add(1)
            .expect("DA Block height overflow: this should be unreachable");
        let high = std::cmp::min(highest_seen_height, head.height()).saturating_add(1);

        // But what if low above head???
        // This is only possible if the earliest seen transition is not a direct descendant of the finalized block
        // Which means bug in another method.
        assert!(
            low < high,
            "Error in `low` earliest_seen={low}, highest_seen={highest_seen_height}, head_height={}",
            head.height()
        );

        match self
            .binary_search_for_fork_point(da_service, head, low, high)
            .await?
        {
            BinarySearchOutcome::Done(result) => Ok(result),
            BinarySearchOutcome::Exhausted {
                low,
                high,
                final_candidate,
            } => Ok(self.handle_exhausted_search(low, high, highest_seen_height, final_candidate)),
        }
    }

    /// Performs binary search to find a fork point in the current chain.
    ///
    /// Returns:
    /// - `Done(result)` if search completes with a definitive result
    /// - `Exhausted { low, high, final_candidate }` if loop exits without finding fork point
    async fn binary_search_for_fork_point(
        &self,
        da_service: &Da,
        mut head: <Da::Spec as DaSpec>::BlockHeader,
        mut low: u64,
        mut high: u64,
    ) -> anyhow::Result<BinarySearchOutcome<Da>> {
        let mut final_candidate_header = None;

        while low <= high {
            let mid = low + (high - low) / 2;
            tracing::trace!(
                candidate_height = mid,
                low,
                high,
                head = %head.display(),
                "Checking height"
            );
            let (candidate_header, this_head) = tokio::try_join!(
                da_service.get_block_header_at(mid),
                da_service.get_head_block_header()
            )?;

            if is_head_changed::<Da::Spec>(&head, &this_head) {
                return Ok(BinarySearchOutcome::Done(
                    ForkPointSearchResult::HeadChanged(this_head),
                ));
            }
            // Update head if another progression happens, we don't return early
            head = this_head;

            if self.is_valid_fork_point(&candidate_header) {
                tracing::trace!(candidate = %candidate_header.display(), "Found a matching candidate");
                return Ok(BinarySearchOutcome::Done(ForkPointSearchResult::Found {
                    height: candidate_header.height(),
                }));
            }

            if self.state_on_block.contains_key(&candidate_header.hash()) {
                // Seen this block, moving right.
                low = mid.saturating_add(1);
                tracing::trace!(
                    candidate = %candidate_header.display(),
                    new_low = low,
                    high = high,
                    "Seen this candidate, trying to find a later one");
            } else {
                // Haven't seen this block, moving left.
                high = mid.saturating_sub(1);
                tracing::trace!(
                    candidate = %candidate_header.display(),
                    low = low,
                    new_high = high,
                    "Block is not a continuation of any seen transitions, checking earlier blocks"
                );
            }

            final_candidate_header = Some(candidate_header);
        }

        tracing::trace!("Haven't found candidate for fork point on seen transitions. It means candidate should be the next after last processed finalized height");

        Ok(BinarySearchOutcome::Exhausted {
            low,
            high,
            final_candidate: final_candidate_header.expect("Loop executed at least once"),
        })
    }

    /// Handles the case when binary search exhausted without finding a fork point.
    ///
    /// NOTE: This entire method could be replaced with:
    ///   `ForkPointSearchResult::NeedFutureBlock { height: low }`
    /// The validation would then happen in `check_continuation()` instead.
    /// Trade-off: one extra network call vs ~40 lines of validation code.
    fn handle_exhausted_search(
        &self,
        low: u64,
        high: u64,
        highest_seen_height: u64,
        final_candidate: <Da::Spec as DaSpec>::BlockHeader,
    ) -> ForkPointSearchResult<Da> {
        assert!(
            high <= highest_seen_height.saturating_add(1),
            "Error in `high` = {high}, it should not be larger than largest seen {highest_seen_height} height by more than 1"
        );
        let earliest_seen_height = self
            .last_processed_finalized_header
            .height()
            .checked_add(1)
            .expect("DA Block height overflow: this should be unreachable");
        assert!(
            low >= earliest_seen_height,
            "low={low} should not be less than {earliest_seen_height}"
        );

        if low == earliest_seen_height {
            tracing::trace!(
                earliest_seen_height,
                highest_seen_height,
                low,
                high,
                "Seen nothing in current chain, will check if lowest block matches"
            );

            assert_eq!(final_candidate.height(), earliest_seen_height);

            // All earliest transitions point to the last known finalized state
            let any_earliest_seen_hash = self
                .seen_on_height
                .first_key_value()
                .expect("Choosing fork point only possible if some transitions have been seen")
                .1
                .iter()
                .next()
                .expect("There should be no entries without values");

            // All earliest seen transitions must have prev_hash pointing to last_processed_finalized_header.
            // If the candidate's prev_hash differs, it means the DA finalized a different block than
            // what our earliest seen transitions descended from - this indicates data corruption or
            // a bug in the survivors' logic.
            let earliest_prev_hash = self.get_prev_hash(any_earliest_seen_hash);
            if earliest_prev_hash != final_candidate.prev_hash() {
                panic!(
                    "Finalized chain inconsistency detected: \
                    earliest seen transition at height {earliest_seen_height} \
                    points to parent hash {earliest_prev_hash}, \
                    but current chain's block at that height has parent hash {}. \
                    last_processed_finalized_header={}, candidate={}",
                    final_candidate.prev_hash(),
                    self.last_processed_finalized_header.display(),
                    final_candidate.display(),
                );
            }

            ForkPointSearchResult::Found {
                height: final_candidate.height(),
            }
        } else {
            // We've seen all blocks in the current chain up to DA head.
            // Instead of waiting for future blocks, return the height runner should fetch.
            // The next height to check is `low` (the first unprocessed height in current fork).
            let next_height = low;
            tracing::trace!(
                earliest_seen_height,
                highest_seen_height,
                low,
                high,
                next_height,
                "Seen everything in current chain, returning height for runner to fetch"
            );

            ForkPointSearchResult::NeedFutureBlock {
                height: next_height,
            }
        }
    }

    /// Determines the effective finalized header to use for processing.
    ///
    /// Handles edge cases where DA layer reports unexpected finalized heights:
    /// - If syncing: finalized might be higher than we've seen - cap at highest_seen
    /// - If DA reports stale: finalized might be lower than already processed - use last_processed
    async fn get_effective_finalized_header(
        &self,
        highest_seen_transition: u64,
    ) -> anyhow::Result<<<Da as DaService>::Spec as DaSpec>::BlockHeader> {
        let last_finalized_header = self
            .finalized_headers_provider
            .get_last_finalized_block_header()?;

        if last_finalized_header.height() > highest_seen_transition {
            // Syncing case: DA is ahead of us
            // TODO: PROBLEM. This can return orphaned (non-finalized) block if:
            //   - we are on chain with reorgs.
            //   - connected RPC node has been switched abruptly between call to `get_last_finalized_block` and now is on part of the chain that is out of sync.
            Ok(self
                .finalized_headers_provider
                .get_block_header_at(highest_seen_transition)
                .await?
                .clone())
        } else if last_finalized_header.height() < self.last_processed_finalized_header.height() {
            // Stale header case: DA reports older than what we've already finalized
            tracing::warn!(
                reported_finalized = last_finalized_header.height(),
                already_processed = self.last_processed_finalized_header.height(),
                "DA layer reported stale finalized header, using last processed instead"
            );
            Ok(self.last_processed_finalized_header.clone())
        } else {
            Ok(last_finalized_header.clone())
        }
    }

    /// Removes all transitions that don't descend from the effective finalized header.
    ///
    /// This is necessary because:
    /// 1. Not all orphaned transitions will be removed naturally, leaving memory waste.
    /// 2. We rely on clean seen state to detect reorgs.
    fn prune_orphaned_transitions(
        &mut self,
        effective_finalized_header: &<<Da as DaService>::Spec as DaSpec>::BlockHeader,
        highest_seen_transition: u64,
    ) {
        let start_height = effective_finalized_header
            .height()
            .checked_add(1)
            .expect("end of chain");

        // Nothing to prune if start is above highest seen
        if start_height > highest_seen_transition {
            return;
        }

        let mut survivors = vec![effective_finalized_header.hash()];

        let range = start_height..=highest_seen_transition;
        tracing::trace!(
            effective_finalized_header = %effective_finalized_header.display(),
            ?range,
            "Pruning transitions not derived from effective finalized header"
        );

        for height in range {
            let new_survivors: Vec<_> = {
                let this_height_blocks = self
                    .seen_on_height
                    .get(&height)
                    .expect("Continuity broken, inconsistent internal state");
                this_height_blocks
                    .iter()
                    .filter(|block_hash| survivors.contains(&self.get_prev_hash(block_hash)))
                    .cloned()
                    .collect()
            };

            self.seen_on_height
                .get_mut(&height)
                .expect("Continuity broken, inconsistent internal state")
                .retain(|block_hash| {
                    if new_survivors.contains(block_hash) {
                        true
                    } else {
                        tracing::trace!(
                            %block_hash,
                            ?new_survivors,
                            "Removing orphaned transition"
                        );
                        self.state_on_block.remove(block_hash);
                        false
                    }
                });

            survivors = new_survivors;
        }
    }

    /// Extracts all finalized transitions from the seen transitions.
    ///
    /// Returns transitions in order from oldest to newest.
    /// Removes extracted transitions from internal state.
    async fn extract_finalized_transitions(
        &mut self,
        effective_finalized_header: &<<Da as DaService>::Spec as DaSpec>::BlockHeader,
        earliest_seen_transition: u64,
    ) -> anyhow::Result<Vec<StateOnBlock<Da::Spec, StateRoot>>> {
        let seen_transitions_to_finalize = effective_finalized_header
            .height()
            .saturating_sub(self.last_processed_finalized_header.height());
        let mut finalized_transitions = Vec::with_capacity(seen_transitions_to_finalize as usize);

        let range = (earliest_seen_transition..=effective_finalized_header.height()).rev();
        tracing::trace!(
            ?range,
            "Extracting finalized transitions from previously seen transitions"
        );

        // This is to safely connect blocks and avoid getting wrong block
        let mut next_hash_to_finalize = effective_finalized_header.hash();
        // This is **reverse** range from finalized down to earliest seen
        for height in range {
            let blocks_on_height = self
                .seen_on_height
                .remove(&height)
                .expect("Should be at least one seen transition on each height");

            assert!(
                blocks_on_height.contains(&next_hash_to_finalize),
                "We haven't seen transition {next_hash_to_finalize} that we suppose to finalize"
            );
            let transition = self
                .state_on_block
                .remove(&next_hash_to_finalize)
                .expect("Internal inconsistency between `seen_on_height` and `state_on_block`");

            next_hash_to_finalize = transition.block_header.prev_hash();
            finalized_transitions.push(transition);
            for block_hash in blocks_on_height {
                self.state_on_block.remove(&block_hash);
            }
        }

        // Remove all entries in `seen_on_height` that have empty vectors.
        self.seen_on_height.retain(|_, entries| !entries.is_empty());

        finalized_transitions.reverse();
        tracing::trace!(
            finalized_transitions = finalized_transitions.len(),
            "Extracted finalized transitions"
        );

        Ok(finalized_transitions)
    }

    /// Returns all [`StateTransitionInfo`] which are below finalized height at this point.
    async fn process_finalized_state_transitions(
        &mut self,
    ) -> anyhow::Result<Vec<StateOnBlock<Da::Spec, StateRoot>>> {
        let earliest_seen = self
            .get_earliest_seen_height()
            .expect("Should be called after at least single transition added");
        let highest_seen = self
            .get_highest_seen_height()
            .expect("Should be called after at least single transition added");

        assert!(
            earliest_seen <= highest_seen,
            "bug in state manager. earliest and highest transition numbers are calculated incorrectly"
        );
        assert_eq!(
            earliest_seen,
            self.last_processed_finalized_header
                .height()
                .saturating_add(1),
            "bug in state manager. earliest seen transition should be incremental from last processed finalized"
        );

        // 1. Determine effective finalized header (handles syncing/stale edge cases)
        let effective_finalized = self.get_effective_finalized_header(highest_seen).await?;
        assert!(
            self.state_on_block
                .contains_key(&effective_finalized.hash())
                || self.last_processed_finalized_header.hash() == effective_finalized.hash(),
            "Effective finalized header must be a block we've processed"
        );

        tracing::trace!(
            last_processed = %self.last_processed_finalized_header.display(),
            effective_finalized = %effective_finalized.display(),
            highest_seen,
            seen_transitions = self.state_on_block.len(),
            "Processing finalized state transitions"
        );

        // 2. Prune orphaned transitions not descending from effective finalized header
        self.prune_orphaned_transitions(&effective_finalized, highest_seen);

        // 3. Extract finalized transitions
        let finalized_transitions = self
            .extract_finalized_transitions(&effective_finalized, earliest_seen)
            .await?;

        // 4. Verify continuity and update state
        verify_finalized_transitions_continuity(
            self.last_processed_finalized_header.hash(),
            &finalized_transitions,
        );

        if let Some(last) = finalized_transitions.last() {
            self.last_processed_finalized_header = last.block_header.clone();
            self.last_processed_finalized_state_root = last.post_state_root.clone();
        }

        self.verify_earliest_seen();

        Ok(finalized_transitions)
    }

    // Checks that all earliest seen transitions point to the same block hash.
    fn verify_earliest_seen(&self) {
        if let Some(earliest_blocks) = self.seen_on_height.first_key_value() {
            let expected_prev_hash = self.last_processed_finalized_header.hash();
            for block_hash in earliest_blocks.1 {
                let actual_prev_hash = self.get_prev_hash(block_hash);
                assert_eq!(
                    actual_prev_hash, expected_prev_hash,
                    "Earliest seen transition {} has prev_hash={}, expected={} (last_processed_finalized_header={})",
                    block_hash,
                    actual_prev_hash,
                    expected_prev_hash,
                    self.last_processed_finalized_header.display(),
                );
            }
        }
    }

    fn pre_validate_transition_witness(
        &self,
        transition_witness: &StateTransitionWitness<StateRoot, Witness, Da::Spec>,
        seen_state_on_block: &StateOnBlock<Da::Spec, StateRoot>,
    ) {
        if let Some(prev_state) = self
            .state_on_block
            .get(&transition_witness.da_block_header.prev_hash())
        {
            assert_eq!(
                prev_state.post_state_root.as_ref(),
                seen_state_on_block.pre_state_root.as_ref(),
                "Incorrect transition received"
            );
            assert_eq!(
                prev_state.block_header.hash(),
                transition_witness.da_block_header.prev_hash(),
                "Mismatch in block hashes after transition",
            );
        }
        match self
            .state_on_block
            .get(&transition_witness.da_block_header.prev_hash())
        {
            None => {
                assert_eq!(
                    transition_witness.initial_state_root.as_ref(),
                    self.last_processed_finalized_state_root.as_ref(),
                    "Wrong transition, its pre_state_root does not match the state in StateManager"
                );
            }
            Some(state_on_prev_block) => {
                assert_eq!(
                    transition_witness.initial_state_root.as_ref(),
                    state_on_prev_block.post_state_root.as_ref(),
                    "Wrong transition, its pre_state_root does not match the current state root of StateManager");
            }
        };
    }

    // Make sure that current ledger pre-state matches pre-state of given transition witness
    fn verify_transition_witness_against_ledger_state(
        &self,
        transition_witness: &StateTransitionWitness<StateRoot, Witness, Da::Spec>,
    ) -> anyhow::Result<()> {
        match self.ledger_db.get_head_state_root()? {
            None => {
                // No state root means we're at genesis - verify that's actually the case
                assert_eq!(
                    transition_witness.da_block_header.height(),
                    self.genesis_da_height,
                    "Ledger should have state root after genesis is completed"
                );
            }
            Some(ledger_state_root) => {
                assert_eq!(
                    ledger_state_root.as_slice(),
                    transition_witness.initial_state_root.as_ref(),
                    "Ledger head state root should match the pre-state of the current transition"
                );
            }
        }
        Ok(())
    }

    // Returns updating time
    async fn update_api_and_ledger_storage(
        &mut self,
        block_header: &<<Da as DaService>::Spec as DaSpec>::BlockHeader,
    ) -> anyhow::Result<std::time::Duration> {
        let start = std::time::Instant::now();
        tracing::trace!(after_block = %block_header.display(), "Sending new Ledger and API storages");
        let (api_storage, ledger_state) = self.storage_manager.create_state_after(block_header)?;

        self.update_channels(api_storage, ledger_state).await?;
        let updating_time = start.elapsed();
        tracing::trace!(after_block = %block_header.display(), time = ?updating_time, "Ledger and API storages have been sent");
        Ok(updating_time)
    }

    fn get_slot_number(&self) -> anyhow::Result<SlotNumber> {
        Ok(self.ledger_db.get_next_items_numbers()?.slot_number)
    }
}

/// Returns true if new head is not same as old or next after old.
fn is_head_changed<Da: DaSpec>(current_head: &Da::BlockHeader, new_head: &Da::BlockHeader) -> bool {
    // Basically same block or directly next one.
    // Not 100% precise.
    // If the chain progresses more than 1 block between candidate selection, it will start again.
    if current_head.hash() == new_head.hash() {
        return false;
    } else if new_head.prev_hash() == current_head.hash() {
        tracing::trace!(
            current_head = %current_head.display(),
            new_head = %new_head.display(),
            "Chain has not switched, but progressed by 1 block"
        );
        return false;
    }
    true
}

fn verify_finalized_transitions_continuity<Da, StateRoot>(
    last_processed_finalized_header_hash: Da::SlotHash,
    finalized_transitions: &[StateOnBlock<Da, StateRoot>],
) where
    Da: DaSpec,
    StateRoot: AsRef<[u8]>,
{
    // First is connected to current
    if let Some(first_processed_finalized_transition) = finalized_transitions.first() {
        assert_eq!(
            last_processed_finalized_header_hash,
            first_processed_finalized_transition
                .block_header
                .prev_hash(),
            "First finalized transition has disconnected block header from StateManager's last_processed_finalized_block"
        );
    }

    // All correctly connected between each other
    for (i, pair) in finalized_transitions.windows(2).enumerate() {
        let prev = &pair[0];
        let next = &pair[1];
        assert_eq!(
            prev.post_state_root.as_ref(),
            next.pre_state_root.as_ref(),
            "Finalized transition {i} has disconnected state roots"
        );
        assert_eq!(
            prev.block_header.hash(),
            next.block_header.prev_hash(),
            "Finalized transition {i} has disconnected block header"
        );
    }
}
