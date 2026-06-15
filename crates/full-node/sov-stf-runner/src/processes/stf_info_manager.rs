#![allow(dead_code)]
use std::marker::PhantomData;
use std::num::NonZero;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail};
use borsh::BorshSerialize;
use rockbound::SchemaBatch;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_db::proof_manager_db::ProofManagerDb;
use sov_db::schema::types::StoredStfInfo;
#[cfg(test)]
use sov_db::test_utils::CrashLocation;
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::aggregated_proof::SerializedAggregatedProof;
use sov_rollup_interface::zk::StateTransitionWitness;
use sov_rollup_interface::ProvableHeightTracker;
use tokio::sync::mpsc;

/// Holds all the necessary data for the creation of a block zk-proof.
#[derive(Serialize, Deserialize)]
#[serde(bound = "StateRoot: Serialize + DeserializeOwned, Witness: Serialize + DeserializeOwned")]
pub struct StateTransitionInfo<StateRoot, Witness, Da: DaSpec> {
    /// Public input to the per-block zk proof.
    pub(crate) data: StateTransitionWitness<StateRoot, Witness, Da>,
    /// Aggregated proofs processed while executing this transition, in verification order.
    pub(crate) aggregated_proofs: Vec<SerializedAggregatedProof>,
}

impl<StateRoot, Witness, Da: DaSpec> StateTransitionInfo<StateRoot, Witness, Da> {
    /// StateTransitionInfo constructor.
    pub fn new(data: StateTransitionWitness<StateRoot, Witness, Da>) -> Self {
        Self {
            data,
            aggregated_proofs: Vec::new(),
        }
    }

    pub(crate) fn slot_number(&self) -> SlotNumber {
        self.data.slot_number
    }

    pub(crate) fn da_block_header(&self) -> &Da::BlockHeader {
        &self.data.da_block_header
    }

    pub(crate) fn initial_state_root(&self) -> &StateRoot {
        &self.data.initial_state_root
    }

    pub(crate) fn witness(self) -> StateTransitionWitness<StateRoot, Witness, Da> {
        self.data
    }
}

/// Materializes STF infos and sends notifications to the associated [`Receiver`].
pub struct Sender<StateRoot, Witness, Da: DaSpec> {
    /// Height of the next `StateTransitionInfo` that should be received by the [`Receiver`].
    /// This value is synchronized with the receiver end of the channel.
    next_height_to_receive: Arc<AtomicU64>,

    /// The next height to send to the [`Receiver`]. This value is not persisted in the database and
    /// is merely used to avoid sending twice the same height notification to the [`Receiver`].
    pub next_height_to_send: SlotNumber,

    // Max number of entries we will keep in the Db, older data will be pruned.
    max_nb_of_infos_in_db: NonZero<u64>,

    /// The notification channel does not contain the actual STF info data,
    /// only the indexes in the Db where the data is stored.
    ///
    /// ## Note
    /// This channel is used to enforce a back-pressure mechanism to ensure that the
    /// [`Receiver`] and the [`Sender`] are not too out-of-sync.
    /// At the end of each slot, the [`Sender`] will send notifications
    /// to the [`Receiver`] for each rollup height that can be proven.
    /// If the [`Sender::notifier`] channel is full, the rollup will halt and wait for the
    /// [`Receiver`] end to catch up.
    ///
    /// It is important to note that this is allowed by the fact that _every transition_ goes through the
    /// channel, and that the receiver processes them individually and sequentially. Otherwise, the
    /// back-pressure assumptions are broken.
    ///
    /// ## Safety
    /// The size of this channel should not be greater than the [`Sender::max_nb_of_infos_in_db`].
    /// Otherwise it would mean that we can have more transitions in the channel than what are present in the Db.
    notifier: mpsc::Sender<SlotNumber>,

    /// The proof manager database for persisting STF info independently from ledger commits.
    proof_manager_db: ProofManagerDb,

    _phantom: PhantomData<(StateRoot, Witness, Da)>,
}

impl<
        StateRoot: DeserializeOwned + Serialize,
        Witness: DeserializeOwned + Serialize,
        Da: DaSpec,
    > Sender<StateRoot, Witness, Da>
{
    /// This method is only called when starting up the stf info manager. It
    /// ensures that the state is correctly set and synchronized.
    ///
    /// The `ledger_head` and `latest_finalized_slot_number` parameters are used to reconcile
    /// ProofManagerDb with the ledger state on startup, ensuring the proof manager never exposes
    /// data beyond the latest finalized slot while still preserving staged future rows.
    pub(crate) async fn startup_notify_about_infos_from_db(
        &mut self,
        ledger_head: SlotNumber,
        latest_finalized_slot_number: SlotNumber,
        max_provable_slot_number: &dyn ProvableHeightTracker,
    ) -> anyhow::Result<()> {
        // Validate and recover ProofManagerDb metadata against the finalized ledger view.
        self.proof_manager_db
            .validate_and_recover_write_height(ledger_head, latest_finalized_slot_number)?;

        // Re-sync in-memory cursors from DB after recovery, because validation may
        // have rewritten metadata (for example when ProofManagerDb was ahead).
        let maybe_db_next_height_to_receive = self.proof_manager_db.get_next_height_to_receive()?;
        let db_next_height_to_receive =
            maybe_db_next_height_to_receive.unwrap_or_else(|| self.next_height_to_receive());
        self.next_height_to_receive
            .store(db_next_height_to_receive.get(), Ordering::SeqCst);
        self.next_height_to_send = db_next_height_to_receive;

        let maybe_write_rollup_height = self.proof_manager_db.get_write_height()?;
        let next_rollup_height_to_receive = db_next_height_to_receive.get();

        let max_provable_slot_number = max_provable_slot_number.max_provable_slot_number();

        match maybe_write_rollup_height {
            Some(write_rollup_height) => {
                let unread_visible_infos = write_rollup_height
                    .get()
                    .saturating_sub(next_rollup_height_to_receive);
                if next_rollup_height_to_receive <= write_rollup_height.get() {
                    assert!(
                        unread_visible_infos <= self.max_nb_of_infos_in_db.get(),
                        "Too many STF infos in the db: {}, vs max allowed {} last_submitted={} write={}",
                        unread_visible_infos,
                        self.max_nb_of_infos_in_db,
                        next_rollup_height_to_receive,
                        write_rollup_height,
                    );
                }
            }
            // Db is empty
            None => {
                assert!(maybe_db_next_height_to_receive.is_none());
                assert!(self.proof_manager_db.get_oldest_height()?.is_none());
            }
        }

        // We notify the receiver about the STF infos available between the last submitted height and the next height to receive.
        // We are only notifying the maximum height that is available in the DB - the `Receiver` will ensure to read every transition
        // between `Receiver::next_height_to_receive` and `max_provable_slot_number`.
        self.notify(max_provable_slot_number).await?;

        Ok(())
    }

    /// Get the next height to receive as a `SlotNumber`.
    pub fn next_height_to_receive(&self) -> SlotNumber {
        SlotNumber::new(self.next_height_to_receive.load(Ordering::SeqCst))
    }

    /// Increment next height to receive by one, returning the previous value.
    pub fn inc_next_height_to_receive(&self) -> SlotNumber {
        SlotNumber::new(self.next_height_to_receive.fetch_add(1, Ordering::SeqCst))
    }
}

/// Receives notifications from the associated [`Sender`] and reads STF info data from the db.
pub struct Receiver<StateRoot, Witness, Da: DaSpec> {
    /// Height of the next `StateTransitionInfo` that is expected to be processed by the `Receiver`
    next_height_to_receive: Arc<AtomicU64>,
    proof_manager_db: ProofManagerDb,
    receiver: mpsc::Receiver<SlotNumber>,
    _phantom: PhantomData<(StateRoot, Witness, Da)>,
}

/// Creates a new [`Sender`] and [`Receiver`] channel.
///
/// The channel's data is retained across Db restarts.
/// - The sender will block if the channel reaches `max_channel_size` of STF infos.
/// - If the number of entries in the Db exceeds `max_nb_of_infos_in_db`, the
///   oldest data will be pruned.
///
/// The channel can only be created if `max_channel_size` is less than or equal
/// to `max_nb_of_infos_in_db`.
#[allow(clippy::type_complexity)]
pub fn new_stf_info_channel<StateRoot, Witness, Da: DaSpec>(
    proof_manager_db: ProofManagerDb,
    max_channel_size: NonZero<u64>,
    max_nb_of_infos_in_db: NonZero<u64>,
    latest_proof_final_slot: Option<SlotNumber>,
) -> anyhow::Result<(
    Sender<StateRoot, Witness, Da>,
    Receiver<StateRoot, Witness, Da>,
)> {
    assert!(
        max_channel_size <= max_nb_of_infos_in_db,
        "Channel size should be smaller than the max number of STFInfos in the db"
    );

    // Internally, the Db keeps the following entries:
    // 1. The STF info data.
    // 2. The highest finalized STF height currently visible to proof-manager consumers.
    // 3. The next height of the retrieved STF info (increased on every `read_next`` operation).

    // On startup, we need to fill the notification channel with the pending STF info from the db.
    let (notifier, receiver) =
        tokio::sync::mpsc::channel::<SlotNumber>(max_channel_size.get().try_into()?);

    let next_height_to_receive = proof_manager_db
        .get_next_height_to_receive()?
        .or_else(|| latest_proof_final_slot.map(|slot| slot.saturating_add(1)))
        .unwrap_or(SlotNumber::ONE);

    let next_height_to_receive_ref = Arc::new(AtomicU64::new(next_height_to_receive.get()));

    let sender = Sender {
        max_nb_of_infos_in_db,
        next_height_to_receive: next_height_to_receive_ref.clone(),
        next_height_to_send: next_height_to_receive,
        notifier,
        proof_manager_db: proof_manager_db.clone(),
        _phantom: PhantomData,
    };

    let receiver = Receiver {
        next_height_to_receive: next_height_to_receive_ref,
        proof_manager_db,
        receiver,
        _phantom: PhantomData,
    };

    Ok((sender, receiver))
}

impl<StateRoot, Witness, Da: DaSpec> Sender<StateRoot, Witness, Da>
where
    StateRoot: Serialize + DeserializeOwned,
    Witness: Serialize + DeserializeOwned,
{
    /// Sends the stf update notifications and updates the last submitted height.
    pub async fn notify(&mut self, max_provable_slot_number: SlotNumber) -> anyhow::Result<()> {
        // The `write_rollup_height` is the highest finalized STF slot visible to consumers.
        let Some(write_rollup_height) = self.proof_manager_db.get_write_height()? else {
            // DB is empty, so we don't have to notify anything.
            return Ok(());
        };

        self.realign_notifier_cursor_to_available_stf_info(write_rollup_height)
            .await?;
        let next_height_to_send = self.next_height_to_send;

        // We always have to ensure we don't notify for a height that is not finalized and visible
        // in the proof-manager metadata, even if later STF rows have already been staged.
        let height_to_notify = std::cmp::min(max_provable_slot_number, write_rollup_height);
        if height_to_notify < next_height_to_send {
            return Ok(());
        }

        let range_to_notify = next_height_to_send.range_inclusive(height_to_notify);
        tracing::trace!(
            start_height = ?next_height_to_send,
            end_height = ?height_to_notify,
            "Heights that are going to be scheduling for proving/attesting."
        );
        for height in range_to_notify {
            tracing::trace!(%height, "Requesting for proving/attesting");
            self.notifier.send(height).await?;
            tracing::trace!(%height, "Notified about stf info height for proving/attesting");
        }

        sov_metrics::track_metrics(|tracker| {
            tracker.submit(super::metrics::ZkStfInfoChannelMetrics {
                channel_depth: self.notifier.max_capacity() - self.notifier.capacity(),
                channel_capacity: self.notifier.max_capacity(),
            });
        });

        if height_to_notify >= next_height_to_send {
            self.next_height_to_send = height_to_notify.saturating_add(1);
        }

        Ok(())
    }

    fn prune_entries(
        &self,
        next_height_to_receive: SlotNumber,
        write_rollup_height: SlotNumber,
    ) -> anyhow::Result<SchemaBatch> {
        let Some(prune_up_to) = write_rollup_height.checked_sub(self.max_nb_of_infos_in_db.get())
        else {
            // If we have not reached [`Self::max_nb_of_infos_in_db`] we don't need to prune the data
            return Ok(Default::default());
        };

        let oldest_height = self.get_oldest_slot_number()?;

        let mut out_schema = SchemaBatch::new();
        for i in oldest_height.range_exclusive(prune_up_to) {
            if i >= next_height_to_receive {
                tracing::warn!(
                    %next_height_to_receive,
                    ?prune_up_to,
                    "State Transition Info is not consumed fast enough, cannot prune older entries. Please check that consumer works."
                );
                break;
            }
            out_schema.merge(self.proof_manager_db.materialize_delete_stf_info(i)?);
        }

        if prune_up_to > oldest_height {
            out_schema.merge(
                self.proof_manager_db
                    .materialize_oldest_height(prune_up_to)?,
            );
        }

        Ok(out_schema)
    }

    /// Builds a SchemaBatch for staging STF info data only (no metadata updates).
    pub fn materialize_stf_info_data(
        &self,
        stf_info: &StateTransitionInfo<StateRoot, Witness, Da>,
    ) -> anyhow::Result<SchemaBatch> {
        let encoded_stf_info: Vec<u8> = bincode::serialize(stf_info).unwrap();
        let stored_stf_info = StoredStfInfo {
            data: encoded_stf_info,
        };

        let write_rollup_height = stf_info.slot_number();

        let schema = self
            .proof_manager_db
            .materialize_stf_info(write_rollup_height, &stored_stf_info)?;

        tracing::trace!(
            %write_rollup_height,
            "Done materializing stf_info data"
        );
        Ok(schema)
    }

    /// Stage STF info data in ProofManagerDb immediately.
    ///
    /// # Two-phase commit pattern
    ///
    /// Because ProofManagerDb and LedgerDb are separate databases, they cannot
    /// be committed atomically. To maintain consistency:
    ///
    /// 1. **Stage** (this method): writes STF info data to ProofManagerDb
    ///    before the ledger commit.
    /// 2. **Commit** ([`Self::commit_stf_info`]): after the ledger commit
    ///    succeeds, advances `write_height` metadata only to the latest finalized slot.
    ///
    /// If a crash occurs between staging and committing,
    /// [`ProofManagerDb::validate_and_recover_write_height`] keeps the staged STF rows and
    /// re-exposes only the contiguous portion that is finalized according to LedgerDb.
    pub async fn stage_stf_info(
        &self,
        stf_info: &StateTransitionInfo<StateRoot, Witness, Da>,
    ) -> anyhow::Result<()> {
        let schema = self.materialize_stf_info_data(stf_info)?;
        self.write_batch_blocking(schema).await?;
        Ok(())
    }

    /// Commit STF info metadata after ledger finality advances.
    pub async fn commit_stf_info(&self, write_rollup_height: SlotNumber) -> anyhow::Result<()> {
        let next_rollup_height_to_receive = self.next_height_to_receive();
        let current_write_rollup_height = self
            .proof_manager_db
            .get_write_height()?
            .unwrap_or(SlotNumber::GENESIS);
        let mut schema = self
            .proof_manager_db
            .materialize_write_height(write_rollup_height)?;

        // Prune the oldest entries if needed
        schema.merge(self.prune_entries(next_rollup_height_to_receive, write_rollup_height)?);

        assert!(
            current_write_rollup_height <= write_rollup_height,
            "write({write_rollup_height}) is smaller than current write_height({current_write_rollup_height})"
        );

        self.write_batch_blocking(schema).await?;
        Ok(())
    }

    async fn write_batch_blocking(&self, schema: SchemaBatch) -> anyhow::Result<()> {
        let db = self.proof_manager_db.clone();
        tokio::task::spawn_blocking(move || db.write_batch(&schema))
            .await
            .map_err(|e| anyhow!("ProofManagerDb write task failed: {e}"))??;
        Ok(())
    }

    fn get_oldest_slot_number(&self) -> anyhow::Result<SlotNumber> {
        let oldest_height = self.proof_manager_db.get_oldest_height()?;
        Ok(oldest_height.unwrap_or(SlotNumber::ONE))
    }

    async fn realign_notifier_cursor_to_available_stf_info(
        &mut self,
        write_rollup_height: SlotNumber,
    ) -> anyhow::Result<()> {
        if self.next_height_to_send > write_rollup_height {
            return Ok(());
        }

        let resume_cursor = self.next_height_to_send;
        for height in resume_cursor.range_inclusive(write_rollup_height) {
            if self.proof_manager_db.get_stf_info(height)?.is_some() {
                if height > resume_cursor {
                    // The consumer's resume cursor (derived from the last
                    // durably-observed proof/attestation, so equal to
                    // `latest_proof_final_slot + 1` in zk mode) sits before the
                    // first STF info this node actually has locally. This is
                    // the expected shape when a node ran without a proof
                    // pipeline and now starts one (e.g. a former replica taking
                    // over leadership): observed proofs advanced the resume
                    // cursor while no local STF infos were written. The slots
                    // in the gap will never be proven by this node.
                    tracing::warn!(
                        resume_cursor = %resume_cursor,
                        first_local_slot = %height,
                        %write_rollup_height,
                        "Realigning STF-info notifier cursor: resume position sits before the first locally-materialized STF info. \
                         Slots in [{resume_cursor}, {height}) are not present locally and will be skipped. \
                         This usually means the node previously ran without the proof pipeline (e.g. as a replica)."
                    );
                    self.next_height_to_send = height;
                    self.next_height_to_receive
                        .fetch_max(height.get(), Ordering::SeqCst);
                }
                return Ok(());
            }
        }

        bail!(
            "The `stf-info-manager` metadata says STF infos exist through slot {write_rollup_height}, \
             but none were found in the notify range starting at {resume_cursor}. \
             This is a bug. Please report it"
        );
    }
}

impl<StateRoot, Witness, Da: DaSpec> Receiver<StateRoot, Witness, Da>
where
    StateRoot: BorshSerialize + Serialize + DeserializeOwned,
    Witness: Serialize + DeserializeOwned,
{
    /// Reads the next [`StateTransitionInfo`] from the Db.
    /// This method will block if the channel is empty. This can happen if the producer of the STF info is slower than the consumer.
    /// Returns `Ok(None)` if the stf info for the given height has already been read and processed.
    /// Returns `anyhow::Error` if the channel is closed.
    pub async fn read_next(
        &mut self,
    ) -> anyhow::Result<Option<StateTransitionInfo<StateRoot, Witness, Da>>> {
        if let Some(slot_number) = self.receiver.recv().await {
            tracing::trace!(%slot_number, "Read next stf info");
            let next_height_to_receive = self.next_height_to_receive();
            tracing::trace!(%next_height_to_receive, %slot_number, "Next height to receive");

            // We need to ensure that the rollup height received is greater than the next expected height to
            // ensure we don't see the same height multiple times.
            if slot_number >= next_height_to_receive {
                let stf_info = self.get(slot_number)?.unwrap_or_else(|| {
                    panic!("The `stf-info-manager` sender notified that the stf height {slot_number} is available but the transition is missing from proof manager DB.
                    Please ensure that the `stf-info-manager` only notifies for heights up to `write_rollup_height`. This is a bug. Please report it")
                });

                return Ok(Some(stf_info));
            }

            return Ok(None);
        }

        bail!("Channel closed. Impossible to read next stf info")
    }

    /// Gets [`StateTransitionInfo`] for the corresponding slot number
    fn get(
        &self,
        slot_number: SlotNumber,
    ) -> anyhow::Result<Option<StateTransitionInfo<StateRoot, Witness, Da>>> {
        let maybe_stored_stf_info = self.proof_manager_db.get_stf_info(slot_number)?;

        if let Some(stored_stf_info) = maybe_stored_stf_info {
            Ok(Some(bincode::deserialize(&stored_stf_info.data[..])?))
        } else {
            Ok(None)
        }
    }

    /// Get the next height to receive as a `SlotNumber`.
    pub fn next_height_to_receive(&self) -> SlotNumber {
        SlotNumber::new(self.next_height_to_receive.load(Ordering::SeqCst))
    }

    /// Increment next height to receive by one, returning the previous value.
    pub fn inc_next_height_to_receive(&self) -> SlotNumber {
        SlotNumber::new(self.next_height_to_receive.fetch_add(1, Ordering::SeqCst))
    }

    /// Increment next height to receive by the requested amount, returning the previous value.
    pub fn inc_next_height_to_receive_by(&self, amount: u64) -> SlotNumber {
        SlotNumber::new(
            self.next_height_to_receive
                .fetch_add(amount, Ordering::SeqCst),
        )
    }

    /// Increment next height to receive by the requested amount and immediately persist it.
    pub fn inc_next_height_to_receive_by_and_persist(
        &self,
        amount: u64,
    ) -> anyhow::Result<SlotNumber> {
        self.cursor_handle()
            .inc_next_height_to_receive_by_and_persist(amount)
    }

    /// Returns a cloneable handle that can advance `next_height_to_receive`
    /// from a different task without requiring access to the full
    /// [`Receiver`]. Used by the proof-aggregation pipeline to keep the
    /// cursor advance co-located with the actual proof publication, while
    /// the channel-draining `read_next` still happens on the intake task.
    pub fn cursor_handle(&self) -> CursorHandle {
        CursorHandle {
            next_height_to_receive: self.next_height_to_receive.clone(),
            proof_manager_db: self.proof_manager_db.clone(),
        }
    }
}

/// A clonable handle over the [`Receiver`]'s `next_height_to_receive` cursor.
///
/// The cursor doubles as the prune cutoff for materialized STF infos
/// (see `prune_entries`), so it MUST only be advanced over slots that the
/// consumer has finished with — either because their proof has been durably
/// published, or because they have been intentionally abandoned (e.g. slots
/// inside a resync window that the prover has decided to skip). Holding this
/// handle in a non-receiver task lets the consumer do that advance without
/// giving the task the rest of the [`Receiver`].
#[derive(Clone)]
pub struct CursorHandle {
    next_height_to_receive: Arc<AtomicU64>,
    proof_manager_db: ProofManagerDb,
}

impl CursorHandle {
    /// Increment next height to receive by the requested amount, returning the previous value.
    pub fn inc_next_height_to_receive_by(&self, amount: u64) -> SlotNumber {
        SlotNumber::new(
            self.next_height_to_receive
                .fetch_add(amount, Ordering::SeqCst),
        )
    }

    /// Increment next height to receive by the requested amount and IMMEDIATELY persist
    /// the new value to disk.
    ///
    /// This method must be called after successful aggregated proof posting to DA.
    /// It fixes the duplicate proof submission bug by ensuring the persisted value
    /// is updated immediately, not waiting for the next ledger commit.
    ///
    /// Returns the previous value.
    pub fn inc_next_height_to_receive_by_and_persist(
        &self,
        amount: u64,
    ) -> anyhow::Result<SlotNumber> {
        let old_value = self.next_height_to_receive.load(Ordering::SeqCst);
        let new_raw_value = old_value
            .checked_add(amount)
            .ok_or_else(|| anyhow!("next_height_to_receive overflowed"))?;
        let new_value = SlotNumber::new(new_raw_value);

        // Persist to disk BEFORE advancing the in-memory atomic.
        // If the write fails, in-memory state remains consistent with disk.
        self.proof_manager_db
            .set_next_height_to_receive(new_value)?;
        #[cfg(test)]
        CrashLocation::AfterPersistingProofManagerNextHeight.crash_if_env_set();

        self.next_height_to_receive
            .store(new_raw_value, Ordering::SeqCst);

        Ok(SlotNumber::new(old_value))
    }
}

#[cfg(test)]
mod tests {
    use std::panic::AssertUnwindSafe;
    use std::path::Path;

    use sov_db::test_utils::{CrashLocation, CRASH_ENV_NAME};
    use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
    use sov_modules_api::da::Time;
    use sov_modules_api::provable_height_tracker::InfiniteHeight;
    use sov_rollup_interface::da::{DaProof, RelevantBlobs, RelevantProofs};
    use sov_rollup_interface::zk::StateTransitionWitness;

    use super::*;
    use crate::processes::StateTransitionInfo;

    type StateRoot = Vec<u8>;
    type Witness = Vec<u8>;

    struct CrashEnvGuard;

    impl CrashEnvGuard {
        fn set(location: CrashLocation) -> Self {
            location.set_crash_env();
            Self
        }
    }

    impl Drop for CrashEnvGuard {
        fn drop(&mut self) {
            std::env::remove_var(CRASH_ENV_NAME);
        }
    }

    #[allow(clippy::type_complexity)]
    fn setup(
        path: &Path,
        max_channel_size: u64,
        max_nb_of_infos_in_db: u64,
    ) -> anyhow::Result<(
        ProofManagerDb,
        Sender<StateRoot, Witness, MockDaSpec>,
        Receiver<StateRoot, Witness, MockDaSpec>,
    )> {
        setup_with_resume(path, max_channel_size, max_nb_of_infos_in_db, None)
    }

    #[allow(clippy::type_complexity)]
    fn setup_with_resume(
        path: &Path,
        max_channel_size: u64,
        max_nb_of_infos_in_db: u64,
        latest_proof_final_slot: Option<SlotNumber>,
    ) -> anyhow::Result<(
        ProofManagerDb,
        Sender<StateRoot, Witness, MockDaSpec>,
        Receiver<StateRoot, Witness, MockDaSpec>,
    )> {
        let proof_manager_db = ProofManagerDb::open(path)?;

        let (sender, receiver) = new_stf_info_channel::<StateRoot, Witness, MockDaSpec>(
            proof_manager_db.clone(),
            NonZero::new(max_channel_size).unwrap(),
            NonZero::new(max_nb_of_infos_in_db).unwrap(),
            latest_proof_final_slot,
        )?;

        Ok((proof_manager_db, sender, receiver))
    }

    async fn setup_with_startup(
        path: &Path,
        max_channel_size: u64,
        max_nb_of_infos_in_db: u64,
        ledger_head: SlotNumber,
    ) -> anyhow::Result<(
        ProofManagerDb,
        Sender<StateRoot, Witness, MockDaSpec>,
        Receiver<StateRoot, Witness, MockDaSpec>,
    )> {
        let (proof_manager_db, mut sender, receiver) =
            setup(path, max_channel_size, max_nb_of_infos_in_db)?;

        sender
            .startup_notify_about_infos_from_db(ledger_head, ledger_head, &InfiniteHeight)
            .await?;

        Ok((proof_manager_db, sender, receiver))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_start_stop_db() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;

        let channel_size = 10;
        let max_nb_of_infos_in_db = 10;

        // Write some data to the Db.
        {
            let (_proof_manager_db, mut sender, _receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::new(channel_size),
            )
            .await?;

            for height in 1..=channel_size {
                let stf_info = make_stf_info(height);
                sender.stage_stf_info(&stf_info).await?;
                sender.commit_stf_info(stf_info.slot_number()).await?;
                sender.notify(stf_info.slot_number()).await?;
            }
        }

        // Restart the Db and check that we can read the previously written data.
        {
            let (_proof_manager_db, sender, mut receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::new(channel_size),
            )
            .await?;

            for i in 1..=channel_size {
                let stf_info = receiver.read_next().await?.unwrap();
                assert_eq!(stf_info.slot_number().get(), i);
            }

            assert_eq!(sender.get_oldest_slot_number()?.get(), 1);
        }

        // We haven't committed the reads above so after restart we will read the same data.
        {
            let (_proof_manager_db, mut sender, mut receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::new(channel_size + 2),
            )
            .await?;

            for i in 1..=channel_size {
                let stf_info = receiver.read_next().await?.unwrap();
                assert_eq!(stf_info.slot_number().get(), i);
            }

            assert_eq!(sender.get_oldest_slot_number()?.get(), 1);

            // Commit new data using the persist method
            receiver.inc_next_height_to_receive_by_and_persist(2)?;

            let stf_info = make_stf_info(channel_size + 1);
            sender.stage_stf_info(&stf_info).await?;
            sender.commit_stf_info(stf_info.slot_number()).await?;
            sender.notify(stf_info.slot_number()).await?;

            let stf_info = make_stf_info(channel_size + 2);
            sender.stage_stf_info(&stf_info).await?;
            sender.commit_stf_info(stf_info.slot_number()).await?;
            sender.notify(stf_info.slot_number()).await?;

            assert_eq!(sender.get_oldest_slot_number()?.get(), 2);
        }

        // Now the reads are visible because we used inc_next_height_to_receive_by_and_persist.
        {
            let (_proof_manager_db, sender, mut receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::new(channel_size + 2),
            )
            .await?;

            let stf_info = receiver.read_next().await?.unwrap();
            assert_eq!(stf_info.slot_number().get(), 3);
            assert_eq!(sender.get_oldest_slot_number()?.get(), 2);
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_restart_realigns_missing_local_prefix() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 10;

        // Simulate a former replica that restarts as a prover: the resume cursor
        // points at slot 3, but the local node only has STF infos starting at 5.
        {
            let (_proof_manager_db, sender, _receiver) = setup_with_resume(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                Some(SlotNumber::new(2)),
            )?;

            for height in 5..=6 {
                let stf_info = make_stf_info(height);
                sender.stage_stf_info(&stf_info).await?;
                sender.commit_stf_info(stf_info.slot_number()).await?;
            }
        }

        {
            let (_proof_manager_db, mut sender, mut receiver) = setup_with_resume(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                Some(SlotNumber::new(2)),
            )?;

            sender
                .startup_notify_about_infos_from_db(
                    SlotNumber::new(6),
                    SlotNumber::new(6),
                    &InfiniteHeight,
                )
                .await?;

            let stf_info = receiver.read_next().await?.unwrap();
            assert_eq!(stf_info.slot_number().get(), 5);

            let stf_info = receiver.read_next().await?.unwrap();
            assert_eq!(stf_info.slot_number().get(), 6);
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_channel() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        let (_proof_manager_db, mut sender, mut receiver) = setup_with_startup(
            temp_dir.path(),
            channel_size,
            max_nb_of_infos_in_db,
            SlotNumber::new(channel_size),
        )
        .await?;

        // Fill the db.
        for height in 1..channel_size {
            let stf_info = make_stf_info(height);
            sender.stage_stf_info(&stf_info).await?;
            sender.commit_stf_info(stf_info.slot_number()).await?;
            sender.notify(stf_info.slot_number()).await?;
        }

        // Read the data from the db.
        for height in 1..channel_size {
            let stf_info = receiver.read_next().await?.unwrap();
            receiver
                .next_height_to_receive
                .fetch_add(1, Ordering::SeqCst);

            assert_eq!(stf_info.slot_number().get(), height);
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_drop_sender() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        let (_proof_manager_db, mut sender, mut receiver) = setup_with_startup(
            temp_dir.path(),
            channel_size,
            max_nb_of_infos_in_db,
            SlotNumber::new(channel_size),
        )
        .await?;

        for height in 1..3 {
            let stf_info = make_stf_info(height);
            sender.stage_stf_info(&stf_info).await?;
            sender.commit_stf_info(stf_info.slot_number()).await?;
            sender.notify(stf_info.slot_number()).await?;
        }

        drop(sender);

        for height in 1..3 {
            let stf_info = receiver.read_next().await?.unwrap();
            receiver
                .next_height_to_receive
                .fetch_add(1, Ordering::SeqCst);

            assert_eq!(stf_info.slot_number().get(), height);
        }

        let stf_info = receiver.read_next().await;
        receiver
            .next_height_to_receive
            .fetch_add(1, Ordering::SeqCst);
        assert!(stf_info.is_err());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_channel_concurrent() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;

        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        let (_proof_manager_db, mut sender, mut receiver) = setup_with_startup(
            temp_dir.path(),
            channel_size,
            max_nb_of_infos_in_db,
            SlotNumber::new(channel_size),
        )
        .await?;

        tokio::spawn(async move {
            // Fill the db.
            for height in 1..=channel_size {
                let stf_info = make_stf_info(height);
                sender.stage_stf_info(&stf_info).await.unwrap();
                sender
                    .commit_stf_info(stf_info.slot_number())
                    .await
                    .unwrap();
                sender.notify(stf_info.slot_number()).await.unwrap();
            }
        });

        // Read the data from the db.
        for height in 1..=channel_size {
            let stf_info = receiver.read_next().await?.unwrap();
            receiver
                .next_height_to_receive
                .fetch_add(1, Ordering::SeqCst);

            assert_eq!(stf_info.slot_number().get(), height);
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_in_db() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;

        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        let (_proof_manager_db, mut sender, mut receiver) = setup_with_startup(
            temp_dir.path(),
            channel_size,
            max_nb_of_infos_in_db,
            SlotNumber::new(channel_size),
        )
        .await?;

        // At the beginning the db should be empty.
        let fetched_stf_info = receiver.get(SlotNumber::ONE)?;
        assert!(fetched_stf_info.is_none());

        // Insert stf info two times.
        assert_stf_in_db(1, &mut sender, &mut receiver).await;
        assert_stf_in_db(2, &mut sender, &mut receiver).await;

        // Check if the first stf is still in the db.
        let fetched_stf_info = receiver.get(SlotNumber::ONE)?;
        assert!(fetched_stf_info.is_some());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_db_pruning() -> anyhow::Result<()> {
        struct TestCase {
            channel_size: u64,
            max_nb_of_infos_in_db: u64,
            nb_of_stf_infos: u64,
        }

        let test_cases = vec![
            TestCase {
                channel_size: 15,
                max_nb_of_infos_in_db: 20,
                nb_of_stf_infos: 30,
            },
            TestCase {
                channel_size: 15,
                max_nb_of_infos_in_db: 15,
                nb_of_stf_infos: 30,
            },
            TestCase {
                channel_size: 1,
                max_nb_of_infos_in_db: 1,
                nb_of_stf_infos: 2,
            },
        ];

        for test_case in test_cases {
            let temp_dir = tempfile::tempdir()?;

            let expected_oldest_height = std::cmp::max(
                test_case.nb_of_stf_infos - test_case.max_nb_of_infos_in_db - 1,
                1,
            );

            let (_proof_manager_db, mut sender, mut receiver) = setup_with_startup(
                temp_dir.path(),
                test_case.channel_size,
                test_case.max_nb_of_infos_in_db,
                SlotNumber::new(test_case.nb_of_stf_infos),
            )
            .await?;

            // Fill the db.
            for height in 1..test_case.nb_of_stf_infos {
                let stf_info = make_stf_info(height);
                sender.stage_stf_info(&stf_info).await?;
                sender.commit_stf_info(stf_info.slot_number()).await?;
                sender.notify(stf_info.slot_number()).await?;
                receiver.read_next().await?.unwrap();
                receiver
                    .next_height_to_receive
                    .fetch_add(1, Ordering::SeqCst);
            }

            let oldest_height = sender.get_oldest_slot_number()?;
            assert_eq!(oldest_height.get(), expected_oldest_height);

            // Check if the old STF infos are pruned.
            for height in 1..test_case.nb_of_stf_infos {
                let stf_info: Option<StateTransitionInfo<Vec<u8>, Vec<u8>, MockDaSpec>> =
                    receiver.get(SlotNumber::new(height))?;

                if height < oldest_height.get() {
                    // The old data was deleted from the Db.
                    assert!(stf_info.is_none());
                } else {
                    assert!(stf_info.is_some());
                }
            }
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_inc_next_height_to_receive_by_and_persist() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        // Write some data and then use persist method
        {
            let (proof_manager_db, sender, receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::new(10),
            )
            .await?;

            // Write some STF info
            for height in 1..=5 {
                let stf_info = make_stf_info(height);
                sender.stage_stf_info(&stf_info).await?;
            }

            // Use the persist method to advance next_height_to_receive
            let old_value = receiver.inc_next_height_to_receive_by_and_persist(3)?;
            assert_eq!(old_value.get(), 1);

            // Verify it's persisted to DB
            let persisted = proof_manager_db.get_next_height_to_receive()?.unwrap();
            assert_eq!(persisted.get(), 4);
        }

        // After restart, the persisted value should still be there
        {
            let (proof_manager_db, _sender, receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::new(10),
            )
            .await?;

            // The next height to receive should be 4 (persisted from previous run)
            assert_eq!(receiver.next_height_to_receive().get(), 4);
            assert_eq!(
                proof_manager_db
                    .get_next_height_to_receive()?
                    .unwrap()
                    .get(),
                4
            );
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_restart_recovers_after_crash_persisting_next_height() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        {
            let (_proof_manager_db, sender, receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::ONE,
            )
            .await?;

            let stf_info = make_stf_info(1);
            sender.stage_stf_info(&stf_info).await?;
            sender.commit_stf_info(stf_info.slot_number()).await?;

            let _crash_guard =
                CrashEnvGuard::set(CrashLocation::AfterPersistingProofManagerNextHeight);
            let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                receiver
                    .inc_next_height_to_receive_by_and_persist(1)
                    .unwrap();
            }));
            assert!(
                result.is_err(),
                "expected crash after persisting next height"
            );
        }

        {
            let (proof_manager_db, mut sender, receiver) =
                setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db)?;
            sender
                .startup_notify_about_infos_from_db(
                    SlotNumber::ONE,
                    SlotNumber::ONE,
                    &InfiniteHeight,
                )
                .await?;

            assert_eq!(receiver.next_height_to_receive(), SlotNumber::new(2));
            assert_eq!(sender.next_height_to_send, SlotNumber::new(2));
            assert_eq!(
                proof_manager_db.get_next_height_to_receive()?,
                Some(SlotNumber::new(2))
            );
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_startup_allows_next_height_to_receive_one_ahead_of_write_height(
    ) -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        {
            let (_proof_manager_db, mut sender, mut receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::ONE,
            )
            .await?;

            let stf_info = make_stf_info(1);
            sender.stage_stf_info(&stf_info).await?;
            sender.commit_stf_info(stf_info.slot_number()).await?;
            sender.notify(stf_info.slot_number()).await?;

            let received = receiver.read_next().await?.unwrap();
            assert_eq!(received.slot_number(), SlotNumber::ONE);
            receiver.inc_next_height_to_receive_by_and_persist(1)?;
        }

        {
            let (_proof_manager_db, sender, receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                SlotNumber::ONE,
            )
            .await?;

            assert_eq!(receiver.next_height_to_receive(), SlotNumber::new(2));
            assert_eq!(sender.next_height_to_send, SlotNumber::new(2));
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_startup_resyncs_in_memory_next_height_after_recovery() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        {
            let proof_manager_db = ProofManagerDb::open(temp_dir.path())?;
            proof_manager_db.set_write_height(SlotNumber::new(10))?;
            proof_manager_db.set_next_height_to_receive(SlotNumber::new(12))?;
        }

        // Re-open channel so stale value is read from db before startup recovery runs.
        let (proof_manager_db, mut sender, receiver) =
            setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db)?;
        assert_eq!(receiver.next_height_to_receive(), SlotNumber::new(12));
        assert_eq!(sender.next_height_to_send, SlotNumber::new(12));

        // Recovery truncates write_height to 7 and next_height_to_receive to 8.
        sender
            .startup_notify_about_infos_from_db(
                SlotNumber::new(7),
                SlotNumber::new(7),
                &InfiniteHeight,
            )
            .await?;

        assert_eq!(
            proof_manager_db.get_write_height()?,
            Some(SlotNumber::new(7))
        );
        assert_eq!(
            proof_manager_db.get_next_height_to_receive()?,
            Some(SlotNumber::new(8))
        );
        assert_eq!(receiver.next_height_to_receive(), SlotNumber::new(8));
        assert_eq!(sender.next_height_to_send, SlotNumber::new(8));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_future_only_bootstrap_accepts_next_height_greater_than_write_height(
    ) -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        {
            let proof_manager_db = ProofManagerDb::open(temp_dir.path())?;
            proof_manager_db.set_write_height(SlotNumber::new(5))?;
            proof_manager_db.set_next_height_to_receive(SlotNumber::new(8))?;
            proof_manager_db.set_oldest_height(SlotNumber::new(8))?;
        }

        let (_proof_manager_db, mut sender, mut receiver) =
            setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db)?;

        sender
            .startup_notify_about_infos_from_db(
                SlotNumber::new(7),
                SlotNumber::new(5),
                &InfiniteHeight,
            )
            .await?;

        assert_eq!(receiver.next_height_to_receive(), SlotNumber::new(8));
        assert_eq!(sender.next_height_to_send, SlotNumber::new(8));

        let stf_info = make_stf_info(8);
        sender.stage_stf_info(&stf_info).await?;
        sender.commit_stf_info(stf_info.slot_number()).await?;
        sender.notify(stf_info.slot_number()).await?;

        let received = receiver.read_next().await?.unwrap();
        assert_eq!(received.slot_number(), SlotNumber::new(8));

        Ok(())
    }

    async fn assert_stf_in_db(
        rollup_height: u64,
        sender: &mut Sender<StateRoot, Witness, MockDaSpec>,
        receiver: &mut Receiver<StateRoot, Witness, MockDaSpec>,
    ) {
        let original_state_transition_info = make_stf_info(rollup_height);

        sender
            .stage_stf_info(&original_state_transition_info)
            .await
            .unwrap();
        sender
            .commit_stf_info(original_state_transition_info.slot_number())
            .await
            .unwrap();
        sender.notify(SlotNumber::new(rollup_height)).await.unwrap();

        let fetched_stf_info = receiver
            .get(SlotNumber::new(rollup_height))
            .unwrap()
            .unwrap();

        assert_eq!(
            get_header_hash(&original_state_transition_info),
            get_header_hash(&fetched_stf_info)
        );
    }

    fn make_stf_info(height: u64) -> StateTransitionInfo<Vec<u8>, Vec<u8>, MockDaSpec> {
        StateTransitionInfo::new(StateTransitionWitness {
            initial_state_root: vec![1, 2, 3],
            final_state_root: vec![3, 4, 5],
            da_block_header: MockBlockHeader {
                prev_hash: [0; 32].into(),
                hash: MockHash([height as u8; 32]),
                height,
                time: Time::now(),
            },
            relevant_proofs: RelevantProofs {
                batch: DaProof {
                    inclusion_proof: Default::default(),
                    completeness_proof: Default::default(),
                },
                proof: DaProof {
                    inclusion_proof: Default::default(),
                    completeness_proof: Default::default(),
                },
            },
            relevant_blobs: RelevantBlobs {
                proof_blobs: vec![],
                batch_blobs: vec![],
            },
            witness: vec![],
            slot_number: SlotNumber::new(height),
        })
    }

    fn get_header_hash(stf_info: &StateTransitionInfo<Vec<u8>, Vec<u8>, MockDaSpec>) -> MockHash {
        stf_info.da_block_header().hash
    }
}
