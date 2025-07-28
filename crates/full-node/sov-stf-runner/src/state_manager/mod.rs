//! All code related to handling storage manager anb ledger.

#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

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

use crate::processes::{Sender as StfInfoSender, StateTransitionInfo};
use crate::query_state_update_info;

const MAX_REORG_FINDING_ATTEMPTS: u8 = 30;

/// Point where rollup execution can be resumed after DA fork happened.
struct ForkPoint<Da: DaService, StateRoot> {
    /// The next block in a new fork, following the last seen transition by the rollup.
    block: Da::FilteredBlock,
    /// Last observed state root before the fork.
    pre_state_root: StateRoot,
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

enum ForkPointSearchResult<Da: DaService, StateRoot> {
    Found(ForkPoint<Da, StateRoot>),
    HeadChanged(<Da::Spec as DaSpec>::BlockHeader),
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
    // `state_root` is tracked so [`StateTransitionWitness`] can have proper `prev_state_root`.
    // Probably it can be saved in variable before "apply_slot" is called,
    // But then the runner needs to know about it and carry it over.
    state_root: StateRoot,
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
    da_polling_interval: std::time::Duration,
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
        initial_state_root: StateRoot,
        state_update_channel: watch::Sender<StateUpdateInfo<Sm::StfState>>,
        stf_info_sender: Option<StfInfoSender<StateRoot, Witness, Da::Spec>>,
        state_height_tracker: Box<dyn ProvableHeightTracker>,
        da_sync_state: Arc<DaSyncState>,
        da_polling_interval: std::time::Duration,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            storage_manager,
            ledger_db,
            state_root: initial_state_root,
            state_on_block: Default::default(),
            seen_on_height: Default::default(),
            state_update_sender: state_update_channel,
            stf_info_sender,
            max_provable_slot_number_tracker: state_height_tracker,
            is_initialized: false,
            da_sync_state,
            da_polling_interval,
        })
    }

    pub(crate) async fn startup(&mut self) -> anyhow::Result<()> {
        if let Some(sender) = &mut self.stf_info_sender {
            // If this state manager uses a channel, it MUST be correctly
            // initialized before usage.
            sender
                .startup_notify_about_infos_from_db(
                    &self.ledger_db,
                    &*self.max_provable_slot_number_tracker,
                )
                .await?;
        }
        self.is_initialized = true;
        Ok(())
    }

    /// Allows reading current state root.
    pub fn get_state_root(&self) -> &StateRoot {
        &self.state_root
    }

    /// Returns an [`HierarchicalStorageManager::StfState`] and a [`DaService::FilteredBlock`] that can be used to continue execution.
    /// If a caller relies on some data from `filtered_block`,
    /// it should be updated after the call of this method.
    /// If a given block continues in the current fork, it is simply returned to the caller.
    /// If reorg happened, it will return block following the last seen transition.
    #[tracing::instrument(skip_all)]
    pub(crate) async fn prepare_storage(
        &mut self,
        mut filtered_block: Da::FilteredBlock,
        da_service: &Da,
    ) -> anyhow::Result<(Sm::StfState, Da::FilteredBlock)> {
        let start = std::time::Instant::now();
        if !self.is_initialized {
            anyhow::bail!(
                "StateManager wasn't initialized. Please call `.startup()` method before using"
            );
        }
        let reorg_happened = self
            .has_reorg_happened(filtered_block.header(), da_service)
            .await?;
        tracing::trace!(reorg_happened, "Checked if reorg happened");

        if reorg_happened {
            let ForkPoint {
                block: new_block,
                pre_state_root,
            } = self.choose_fork_point(da_service).await?;
            tracing::trace!(
                old_block = %filtered_block.header().display(),
                new_block = %new_block.header().display(),
                old_pre_state_root = hex::encode(self.state_root.as_ref()),
                new_pre_state_root = hex::encode(pre_state_root.as_ref()),
                "Reorg happened, updating variables");

            // Self check
            {
                if let Some(prev_state) = self.state_on_block.get(&new_block.header().prev_hash()) {
                    assert_eq!(
                        prev_state.post_state_root.as_ref(),
                        pre_state_root.as_ref(),
                        "mismatch in roots after transition"
                    );
                    assert_eq!(
                        prev_state.block_header.hash(),
                        new_block.header().prev_hash(),
                        "Mismatch in block hashes after transition",
                    );
                    assert!(
                        !self.state_on_block.contains_key(&new_block.header().hash()),
                        "We are return already seen block. how come?"
                    );
                }
                assert!(
                    !self.state_on_block.contains_key(&new_block.header().hash()),
                    "trying to return previously seen state"
                );
            }
            tracing::info!(
                old_blok = %filtered_block.header().display(),
                new_block = %new_block.header().display(),
                time = ?start.elapsed(),
                "Chosen fork point"
            );
            filtered_block = new_block;
            self.state_root = pre_state_root;
        }

        let (stf_pre_state, ledger_state) = self
            .storage_manager
            .create_state_for(filtered_block.header())?;
        // Second condition, we only update channels with new state before returning in case of reorg
        if reorg_happened {
            tracing::trace!(
                "Reorg has happened, updating API and Ledger storage before returning Stf state"
            );
            // In case if reorg happened, we want to keep ledger and API storages in sync.
            // Otherwise, the API storage and LedgerDb have been updated in [`Self::update_api_and_ledger_storage`]
            self.update_channels(stf_pre_state.clone(), ledger_state)
                .await?;
        }

        tracing::trace!(
            block_header = %filtered_block.header().display(),
            reorg_happened,
            time = ?start.elapsed(),
            "Returning STF state for block");
        Ok((stf_pre_state, filtered_block))
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
        da_service: &Da,
        da_height_at_genesis: u64,
        stf_changes: Sm::StfChangeSet,
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
        let slot_number = self.get_slot_number()?;
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
            %slot_number,
            current_state_root = hex::encode(self.get_state_root().as_ref()),
            next_state_root = hex::encode(new_state_root.as_ref()),
            aggregated_proofs = aggregated_proofs_count,
            "Saving changes after applying slot"
        );

        // ---
        let seen_state_on_block =
            StateOnBlock::from_transition_witness::<Witness>(&transition_witness);
        tracing::trace!(block_header = %block_header.display(), "Adding transition to the list of the seen");
        // Self check
        {
            if let Some(prev_state) = self.state_on_block.get(&block_header.prev_hash()) {
                assert_eq!(
                    prev_state.post_state_root.as_ref(),
                    seen_state_on_block.pre_state_root.as_ref(),
                    "Incorrect transition received"
                );
                assert_eq!(
                    prev_state.block_header.hash(),
                    block_header.prev_hash(),
                    "Mismatch in block hashes after transition",
                );
            }
        }
        self.state_on_block
            .insert(block_header.hash(), seen_state_on_block);
        self.seen_on_height
            .entry(block_header.height())
            .or_default()
            .insert(block_header.hash());
        // ----

        let processing_finalized_transitions_start = std::time::Instant::now();
        let (last_finalized_header, finalized_transitions) =
            self.process_finalized_state_transitions(da_service).await?;
        let processing_finalized_transitions_time =
            processing_finalized_transitions_start.elapsed();
        tracing::trace!(
            finalized_transitions = finalized_transitions.len(),
            time = ?processing_finalized_transitions_time,
            "Processed finalized transitions"
        );

        let ledger_materialization_start = std::time::Instant::now();
        let mut ledger_change_set = self
            .ledger_db
            .materialize_slot(slot_commit, new_state_root.as_ref())?;
        tracing::trace!("Initial Ledger ChangeSet is materialized");

        // TODO: Review this, does not look nice.
        let last_finalized_slot_number = SlotNumber::new_dangerous(
            last_finalized_header
                .height()
                .saturating_sub(da_height_at_genesis),
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
            "Last finalized slot is materialized into LedgerDb ChangeSet"
        );

        if let Some(stf_info_sender) = &self.stf_info_sender {
            tracing::trace!("Going to materialize StateTransitionInfo");
            let stf_info = StateTransitionInfo {
                data: transition_witness,
                slot_number,
            };
            let stf_info_schema = stf_info_sender
                .materialize_stf_info(&stf_info, &self.ledger_db)
                .await?;
            ledger_change_set.merge(stf_info_schema);
            tracing::trace!("StateTransitionInfo is materialized into Ledger ChangeSet");
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

        let updating_api_time = self.update_api_and_ledger_storage(&block_header).await?;

        let sending_to_prover_start = std::time::Instant::now();
        if let Some(stf_info_sender) = &mut self.stf_info_sender {
            // Notify `StateTransitionInfo` consumers that the data is saved in the Db.
            let max_provable_slot_number = self
                .max_provable_slot_number_tracker
                .max_provable_slot_number();
            tracing::trace!(
                ?max_provable_slot_number,
                "Going to notify stf_info_sender about max provable slot"
            );
            stf_info_sender
                .notify(max_provable_slot_number, &self.ledger_db)
                .await?;
            tracing::trace!(
                ?max_provable_slot_number,
                "State transition info receiver has been notified about max provable slot number"
            );
        }
        let sending_to_prover_time = sending_to_prover_start.elapsed();

        self.state_root = new_state_root;
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
        let state_update_info = query_state_update_info(&self.ledger_db, stf_state).await?;

        // `send_replace` is superior to `send` for our use case. It never fails
        // because it doesn't need to notify all receivers, unlike `send`, which
        // we don't need. It will also keep working even if there are no
        // receivers currently alive, which makes it easier to reason about the
        // code.
        self.state_update_sender.send_replace(state_update_info);

        Ok(())
    }

    /// Returns true, if passing block_header is not an incremental continuation of the current canonical chain.
    async fn has_reorg_happened(
        &self,
        block_header: &<Da::Spec as DaSpec>::BlockHeader,
        da_service: &Da,
    ) -> anyhow::Result<bool> {
        // Reorg: if passed block header a new and it is not a continuation of any of the previous height transitions.
        tracing::trace!(
            block_header = %block_header.display(),
            "Checking if reorg happened");
        // 0. Short circuit
        if self.state_on_block.is_empty() {
            tracing::trace!("empty state_on_block => checking if passed block is finalized or direct descendant of finalized");
            let finalized = da_service.get_last_finalized_block_header().await?;
            // Simple case
            if block_header.prev_hash() == finalized.hash()
                || block_header.hash() == finalized.hash()
            {
                return Ok(false);
            }
            if block_header.height() >= finalized.height() {
                tracing::trace!(
                    block_header = %block_header.display(),
                    last_finalzied = %finalized.display(),
                    "passed block header is higher than finalized and not direct descendant of finalized => reorg happened");
                return Ok(true);
            }
            // If it is not last finalized, but finalized in the past
            let past_finalized_block = da_service
                .get_block_header_at(block_header.height())
                .await?;

            if block_header.hash() == past_finalized_block.hash() {
                tracing::trace!("Passed block header has been finalized in the past => no reorg");
                return Ok(false);
            }
            tracing::trace!(
                "This block header has not been finalized in the past => reorg happened"
            );
            return Ok(false);
        }

        let predecessor_state_root = match self.get_pre_state_root_if_fit_candidate(block_header) {
            None => return Ok(true),
            Some(state_root) => state_root,
        };
        // 3. Continuation of **existing** state of state manager.
        let is_fork = self.state_root.as_ref() != predecessor_state_root.as_ref();
        tracing::trace!(block_header = %block_header.display(), is_fork, "current state matches predecessor");
        Ok(is_fork)
    }

    // Returns preceding state root
    // if given block header is a new continuous transition from seen transition.
    fn get_pre_state_root_if_fit_candidate(
        &self,
        block_header: &<Da::Spec as DaSpec>::BlockHeader,
    ) -> Option<StateRoot> {
        // 1. Has been seen: not a new transition
        if self.state_on_block.contains_key(&block_header.hash()) {
            tracing::trace!(block_header = %block_header.display(), "has been seen => fork");
            return None;
        }

        // 2. Does not have a predecessor: not continuous transition
        self.state_on_block
            .get(&block_header.prev_hash())
            .map(|state| state.post_state_root.clone())
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
    async fn choose_fork_point(&self, da_service: &Da) -> anyhow::Result<ForkPoint<Da, StateRoot>> {
        if self.state_on_block.is_empty() {
            let last_finalized = da_service.get_last_finalized_block_header().await?;
            let adjacent = da_service.get_block_at(last_finalized.height() + 1).await?;
            // reorg can happen between these 2 calls, right now just panic, improve handling in the future.
            // TODO: This can be iterated and included in attempts.
            assert!(adjacent.header().prev_hash() == last_finalized.hash());
            return Ok(ForkPoint {
                block: adjacent,
                pre_state_root: self.state_root.clone(),
            });
        }

        let earliest_seen_height = self
            .get_earliest_seen_height()
            .expect("Choosing fork point only possible if some transitions have been seen");
        let highest_seen_height = self
            .get_highest_seen_height()
            .expect("Choosing fork point only possible if some transitions have been seen");

        let mut head = da_service.get_head_block_header().await?;

        for attempt in 0..MAX_REORG_FINDING_ATTEMPTS {
            match self
                .try_find_candidate_in_current_chain(
                    da_service,
                    head.clone(),
                    earliest_seen_height,
                    highest_seen_height,
                )
                // We could've handle error case and try again, but this is not our responsibility
                .await?
            {
                ForkPointSearchResult::Found(fork_point) => {
                    tracing::trace!(
                        attempt,
                        fork_point = %fork_point.block.header().display(),
                        "Found a candidate for fork point"
                    );
                    return Ok(fork_point);
                }
                ForkPointSearchResult::HeadChanged(new_head) => {
                    tracing::warn!(
                        old_head = %head.display(),
                        new_head = %new_head.display(),
                        attempt,
                        "Reorg happened during fork point selection, trying again"
                    );
                    head = new_head;
                }
            }
        }

        anyhow::bail!("Could find fork point after {MAX_REORG_FINDING_ATTEMPTS} attempts")
    }

    // Tries to find a candidate in the current state of the chain.
    // If it notices that the chain has changed, it returns head of the new chain.
    async fn try_find_candidate_in_current_chain(
        &self,
        da_service: &Da,
        mut head: <Da::Spec as DaSpec>::BlockHeader,
        earliest_seen_height: u64,
        highest_seen_height: u64,
    ) -> anyhow::Result<ForkPointSearchResult<Da, StateRoot>> {
        let mut low = earliest_seen_height;
        let mut high = std::cmp::min(highest_seen_height, head.height()).saturating_add(1);

        // But what if low above head???
        // This is only possible if the earliest seen transition is not a direct descendant of the finalized block
        // Which means bug in another method.

        assert!(
            low < high,
            "Error in `low` earliest_seen={}, highest_seen={}, head_height={} ",
            earliest_seen_height,
            highest_seen_height,
            head.height()
        );

        let mut final_candidate = None;

        while low <= high {
            let mid = low + (high - low) / 2;
            tracing::trace!(
                candidate_height = mid,
                low,
                high,
                earliest_seen_height,
                head = %head.display(),
                "Checking height"
            );
            let (candidate, this_head) = tokio::try_join!(
                da_service.get_block_at(mid),
                da_service.get_head_block_header()
            )?;

            if is_head_changed::<Da::Spec>(&head, &this_head) {
                return Ok(ForkPointSearchResult::HeadChanged(this_head));
            }
            // Update head if another progression happens, we don't return early
            head = this_head;
            if let Some(pre_state_root) =
                self.get_pre_state_root_if_fit_candidate(candidate.header())
            {
                tracing::trace!(candidate = %candidate.header().display(), "Found a matching candidate:");
                return Ok(ForkPointSearchResult::Found(ForkPoint {
                    block: candidate,
                    pre_state_root,
                }));
            }

            if self.state_on_block.contains_key(&candidate.header().hash()) {
                // Seen this block, moving right.
                low = mid.saturating_add(1);
                tracing::trace!(
                    candidate = %candidate.header().display(),
                    new_low = low,
                    high = high,
                    "Seen this candidate, trying to find a later one");
            } else {
                // Haven't seen this block, moving left.
                high = mid.saturating_sub(1);
                tracing::trace!(
                    candidate = %candidate.header().display(),
                    low = low,
                    new_high = high,
                    "Block is not a continuation of any seen transitions, checking earlier blocks"
                );
            }

            final_candidate = Some(candidate);
        }
        tracing::trace!("Haven't found candidate for fork point on seen transitions. It means candidate should be the next after last finalized height");
        // The difference in this case with the loop above,
        // is that we check that block at earliest seen transition height also points to last finalized height.

        assert!(
            high <= highest_seen_height.saturating_add(1),
            "Error in `high`"
        );
        assert!(low >= earliest_seen_height, "Error in `low`");

        if low == earliest_seen_height {
            tracing::trace!(
                earliest_seen_height,
                highest_seen_height,
                low,
                high,
                "Seen nothing in current chain, will check if lowest block matches"
            );
            let candidate = final_candidate.expect("Should be set");

            assert_eq!(candidate.header().height(), earliest_seen_height);

            // All earliest transitions point to the last known finalized state
            let any_earliest_seen_hash = self
                .seen_on_height
                .first_key_value()
                .expect("Choosing fork point only possible if some transitions have been seen")
                .1
                .iter()
                .next()
                .expect("There should be no entries without values");

            if self.get_prev_hash(any_earliest_seen_hash) != candidate.header().prev_hash() {
                tracing::trace!(candidate = %candidate.header().display(), any_earliest = %any_earliest_seen_hash, "There should be block after last finalized");
                panic!("Finalized header changed");
            }

            let state_on_the_same_block = self
                .state_on_block
                .get(any_earliest_seen_hash)
                .expect("Internal inconsistency in maps");
            // As they point to the same previous block, it is safe to return pre_state_root
            let seen_prev_state_root = state_on_the_same_block.pre_state_root.clone();
            Ok(ForkPointSearchResult::Found(ForkPoint {
                block: candidate,
                pre_state_root: seen_prev_state_root,
            }))
        } else {
            tracing::trace!(
                earliest_seen_height,
                highest_seen_height,
                low,
                high,
                "Seen everything in current chain, will wait for next block"
            );

            // This candidate obviously is not fit, otherwise it would've been selected in the main loop
            let mut candidate = final_candidate.expect("Should be set");
            assert_eq!(
                candidate.header().height(),
                high,
                "Wrong candidate for the future block",
            );

            // So we are start going into the future, until we see some block.
            // We will panic if we reach end of the seen heights without finding our candidate.
            // If chain reorgs, we will start over
            loop {
                let next_candidate_height = candidate
                    .header()
                    .height()
                    .checked_add(1)
                    .expect("end of chain");
                let (this_candidate, this_head) = tokio::try_join!(
                    // Need fetch re-org aware if the chain rewinds here.
                    crate::da_utils::fetch_block_reorg_aware(
                        da_service,
                        self.da_sync_state.as_ref(),
                        next_candidate_height,
                        self.da_polling_interval,
                    ),
                    da_service.get_head_block_header(),
                )?;
                if is_head_changed::<Da::Spec>(&head, &this_head) {
                    return Ok(ForkPointSearchResult::HeadChanged(this_head));
                }
                candidate = this_candidate;
                if let Some(pre_state_root) =
                    self.get_pre_state_root_if_fit_candidate(candidate.header())
                {
                    return Ok(ForkPointSearchResult::Found(ForkPoint {
                        block: candidate,
                        pre_state_root,
                    }));
                }
                assert!(self.state_on_block.contains_key(&this_head.hash()), "bug in internal struct. Newly received head hasn't been seen and didn't fit for candidate");
                assert!(self.state_on_block.contains_key(&candidate.header().hash()), "bug in internal struct. Newly received candidate hasn't been seen and didn't fit for candidate");
                head = this_head;
            }
        }
    }

    /// Returns all [`StateTransitionInfo`] which are below finalized height
    /// and relevant LedgerDb changes.
    /// Also returns the latest finalized block header, so caller shouldn't do another call.
    async fn process_finalized_state_transitions(
        &mut self,
        da_service: &Da,
    ) -> anyhow::Result<(
        <Da::Spec as DaSpec>::BlockHeader,
        Vec<StateOnBlock<Da::Spec, StateRoot>>,
    )> {
        // DaService call # 1
        let mut da_service_calls = 1;
        let last_finalized_header = da_service.get_last_finalized_block_header().await?;
        let earliest_seen_transition = self
            .get_earliest_seen_height()
            .expect("Should be called after at least single transition added");
        let highest_seen_transition = self
            .get_highest_seen_height()
            .expect("Should be called after at least single transition added");

        tracing::trace!(
            last_finalized_header = %last_finalized_header.display(),
            highest_seen_transition,
            "Compare truly last finalized header with highest seen transition");

        let last_seen_finalized_header = if last_finalized_header.height() > highest_seen_transition
        {
            // DaService call # 2
            da_service_calls += 1;
            da_service
                .get_block_header_at(highest_seen_transition)
                .await?
                .clone()
        } else {
            last_finalized_header.clone()
        };

        tracing::trace!(
            last_seen_finalized_header = %last_seen_finalized_header.display(),
            seen_transitions = self.state_on_block.len(),
            "Start processing finalized state transitions"
        );

        // Start with eliminating all non-finalized transitions
        // that does not originate from a finalized header.
        // But do we need this? Won't they be cleared on the next iteration, when finalized height rises?
        // Yes, 2 reasons:
        //   1. Not all of them will be removed, so we might have many orphaned transitions in memory.
        //   2. We rely on check on clean-seen state to check if reorg happened or not.
        {
            // We start from height after the last finalized header
            let start_height = last_seen_finalized_header
                .height()
                .checked_add(1)
                .expect("end of chain");
            let mut survivors = vec![last_seen_finalized_header.hash()];

            let range = start_height..=highest_seen_transition;
            tracing::trace!(
                 last_seen_finalized_header = % last_seen_finalized_header.display(),
                ?range,
                "Going to eliminate all future transitions which are not derived from last seen finalized header");
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

                {
                    self.seen_on_height.get_mut(&height)
                            .expect("Continuity broken, inconsistent internal state")
                            .retain(|block_hash| {
                                if new_survivors.contains(block_hash) {
                                    true
                                } else {
                                    tracing::trace!(
                                %block_hash,
                                ?new_survivors,
                                "Removing block header from seen_on_height, because it does not originate from last seen finalized header"
                            );
                                    self.state_on_block.remove(block_hash);
                                    false
                                }
                            });
                }

                survivors = new_survivors;
            }
        }

        let mut finalized_transitions = Vec::with_capacity(
            last_seen_finalized_header
                .height()
                .saturating_sub(earliest_seen_transition) as usize,
        );

        // Going backwards does not mean there's a connection between earliest and fetched last seen finalized header.
        // TO BE 100% sure, we need to do N queries from earliest to latest seen finalized header.
        // We can offload that into a background task that is subscribed and read it via a channel.
        // But now it does queries. In reality, there shouldn't be a lot of them, as normally rollup progresses together with chain.
        let range = (earliest_seen_transition..=last_seen_finalized_header.height()).rev();
        tracing::trace!(
            ?range,
            "Going to extract finalized transitions from previously seen transitions"
        );
        for height in range {
            // DaService call # 3 + n
            let finalized_at_that_height = da_service.get_block_header_at(height).await?;
            da_service_calls += 1;

            tracing::trace!(height, "Going to extract finalized transitions from height");
            let blocks_on_height = self
                .seen_on_height
                .remove(&height)
                .expect("Should be at least one seen transition on each height");
            tracing::trace!(
                ?blocks_on_height,
                "Going to extract finalized transitions from height"
            );
            let mut pushed_for_this_height = false;
            for block_hash in blocks_on_height {
                let transition = self
                    .state_on_block
                    .remove(&block_hash)
                    .expect("Should be there");
                if block_hash == finalized_at_that_height.hash() {
                    assert!(
                        !pushed_for_this_height,
                        "Should be only one finalized transition per height"
                    );
                    finalized_transitions.push(transition);
                    pushed_for_this_height = true;
                }
            }
        }

        // Remove all entries in `seen_on_height` that have empty vectors.
        self.seen_on_height.retain(|_, entries| !entries.is_empty());

        finalized_transitions.reverse();
        tracing::trace!(
            finalized_transitions = finalized_transitions.len(),
            ?da_service_calls,
            "Completed check for finalized transitions"
        );
        Ok((last_finalized_header, finalized_transitions))
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
        tracing::trace!(
            after_block = %block_header.display(),
            time = ?
            "Ledger and API storages have been sent");
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
