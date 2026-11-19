#![allow(dead_code)]
use std::collections::HashMap;
use std::marker::PhantomData;
use std::num::NonZero;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail};
use borsh::BorshSerialize;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sov_db::proof_manager_db::ProofManagerDb;
use sov_db::schema::types::{DbHash, StoredStfInfo};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
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
    /// channel, and that the receiver processes them individually and sequencially. Otherwise, the
    /// back-pressure assumptions are broken.
    ///
    /// ## Safety
    /// The size of this channel should not be greater than the [`Sender::max_nb_of_infos_in_db`].
    /// Otherwise it would mean that we can have more transitions in the channel than what are present in the Db.
    notifier: mpsc::Sender<SlotNumber>,

    /// STF infos of executed but not yet finalized blocks, keyed by DA block hash so that
    /// competing forks at the same slot don't overwrite each other. Only the finalized fork is
    /// written to the [`ProofManagerDb`].
    pending: HashMap<DbHash, (SlotNumber, StoredStfInfo)>,

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
    pub(crate) async fn startup_notify_about_infos_from_db(
        &mut self,
        max_provable_slot_number: &dyn ProvableHeightTracker,
    ) -> anyhow::Result<()> {
        let maybe_write_rollup_height = self.proof_manager_db.latest_slot()?;

        let max_provable_slot_number = max_provable_slot_number.max_provable_slot_number();

        // An empty Db has nothing to realign.
        if let Some(write_rollup_height) = maybe_write_rollup_height {
            self.realign_notifier_cursor_to_available_stf_info(write_rollup_height)?;
            let next_rollup_height_to_receive = self.next_height_to_receive.load(Ordering::SeqCst);
            assert!(
                write_rollup_height.get() + 1 >= next_rollup_height_to_receive,
                "The `write_rollup_height` ({write_rollup_height}) is more than one slot behind `next_rollup_height_to_receive` ({next_rollup_height_to_receive})"
            );

            let outstanding = write_rollup_height
                .get()
                .saturating_sub(next_rollup_height_to_receive);
            assert!(
                outstanding <= self.max_nb_of_infos_in_db.get(),
                "Too many STF infos in the db: {}, vs max allowed {} last_submitted={} write={}",
                outstanding,
                self.max_nb_of_infos_in_db,
                next_rollup_height_to_receive,
                write_rollup_height,
            );
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
pub async fn new_stf_info_channel<StateRoot, Witness, Da: DaSpec>(
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

    // On startup, we need to fill the notification channel with the pending STF info from the db.
    let (notifier, receiver) =
        tokio::sync::mpsc::channel::<SlotNumber>(max_channel_size.get().try_into()?);

    // Resume STF-info processing from the last aggregated proof that was
    // verified on-chain and persisted in the DB: that proof's `final_slot` is
    // the last slot we know is committed, so the prover picks up at
    // `final_slot + 1`. If no such proof exists yet (fresh node), start from
    // genesis.
    let next_height_to_receive = latest_proof_final_slot
        .map(|final_slot| final_slot.saturating_add(1))
        .unwrap_or(SlotNumber::ONE);

    let next_height_to_receive_ref = Arc::new(AtomicU64::new(next_height_to_receive.get()));

    let sender = Sender {
        max_nb_of_infos_in_db,
        next_height_to_receive: next_height_to_receive_ref.clone(),
        next_height_to_send: next_height_to_receive,
        notifier,
        pending: HashMap::new(),
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
        // The `write_rollup_height` is the maximum stf height that is available in the DB.
        let Some(write_rollup_height) = self.proof_manager_db.latest_slot()? else {
            // DB is empty, so we don't have to notify anything.
            return Ok(());
        };

        self.realign_notifier_cursor_to_available_stf_info(write_rollup_height)?;
        let next_height_to_send = self.next_height_to_send;

        // We always have to ensure we don't notify for a height that is not already written to the DB.
        // So we always notify up to the `write_rollup_height`, even if the `max_provable_slot_number` is higher.
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

    /// Returns the slot below which STF infos are pruned, if any. Never prunes at or above
    /// `next_height_to_receive`, so unconsumed STF infos survive.
    fn prune_below(&self, write_rollup_height: SlotNumber) -> Option<SlotNumber> {
        // If we have not reached [`Self::max_nb_of_infos_in_db`] we don't need to prune the data
        let prune_up_to = write_rollup_height.checked_sub(self.max_nb_of_infos_in_db.get())?;
        let next_height_to_receive = self.next_height_to_receive();
        if next_height_to_receive < prune_up_to {
            tracing::warn!(
                %next_height_to_receive,
                ?prune_up_to,
                "State Transition Info is not consumed fast enough, cannot prune older entries. Please check that consumer works."
            );
        }
        Some(prune_up_to.min(next_height_to_receive))
    }

    /// Keeps [`StateTransitionInfo`] of an executed block in memory until its block is finalized.
    pub fn stage_stf_info(&mut self, stf_info: &StateTransitionInfo<StateRoot, Witness, Da>) {
        let stored_stf_info = StoredStfInfo {
            data: bincode::serialize(stf_info).unwrap(),
        };
        self.pending.insert(
            stf_info.da_block_header().hash().into(),
            (stf_info.slot_number(), stored_stf_info),
        );
    }

    /// Writes the staged STF infos of the finalized blocks to the Db, prunes the oldest entries
    /// and drops the staged STF infos of orphaned forks.
    ///
    /// Must be called before the `LedgerDb` finalizes these blocks: after a crash in between,
    /// the node re-executes them and rewrites the same rows.
    pub fn write_finalized_stf_infos(
        &mut self,
        finalized_block_hashes: impl IntoIterator<Item = DbHash>,
    ) -> anyhow::Result<()> {
        let infos = finalized_block_hashes
            .into_iter()
            .map(|hash| {
                self.pending.remove(&hash).ok_or_else(|| {
                    anyhow!(
                        "STF info of finalized block {} was not staged",
                        hex::encode(hash)
                    )
                })
            })
            .collect::<anyhow::Result<Vec<_>>>()?;
        let Some(&(write_rollup_height, _)) = infos.last() else {
            return Ok(());
        };
        // Every block staged at or below the finalized slot is on an orphaned fork now.
        self.pending
            .retain(|_, (slot_number, _)| *slot_number > write_rollup_height);

        let next_rollup_height_to_receive = self.next_height_to_receive();
        // Blocks re-executed after a crash rewrite rows that the consumer may have already read.
        let is_rewrite = self.proof_manager_db.latest_slot()? >= Some(write_rollup_height);
        assert!(
            is_rewrite || next_rollup_height_to_receive <= write_rollup_height,
            "write({write_rollup_height}) is smaller than next height to receive({next_rollup_height_to_receive})"
        );

        self.proof_manager_db
            .write_stf_infos(&infos, self.prune_below(write_rollup_height))?;

        tracing::trace!(
            %next_rollup_height_to_receive,
            %write_rollup_height,
            "Done writing stf_info"
        );
        Ok(())
    }

    fn realign_notifier_cursor_to_available_stf_info(
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
                    panic!("The `stf-info-manager` sender notified that the stf height {slot_number} is available but the transition is missing from the proof manager DB.
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

    /// Returns a cloneable handle that can advance `next_height_to_receive`
    /// from a different task without requiring access to the full
    /// [`Receiver`]. Used by the proof-aggregation pipeline to keep the
    /// cursor advance co-located with the actual proof publication, while
    /// the channel-draining `read_next` still happens on the intake task.
    pub fn cursor_handle(&self) -> CursorHandle {
        CursorHandle {
            next_height_to_receive: self.next_height_to_receive.clone(),
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
}

impl CursorHandle {
    /// Increment next height to receive by the requested amount, returning the previous value.
    pub fn inc_next_height_to_receive_by(&self, amount: u64) -> SlotNumber {
        SlotNumber::new(
            self.next_height_to_receive
                .fetch_add(amount, Ordering::SeqCst),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
    use sov_modules_api::da::Time;
    use sov_modules_api::provable_height_tracker::InfiniteHeight;
    use sov_rollup_interface::da::{DaProof, RelevantBlobs, RelevantProofs};
    use sov_rollup_interface::zk::StateTransitionWitness;

    use super::*;
    use crate::processes::StateTransitionInfo;

    type StateRoot = Vec<u8>;
    type Witness = Vec<u8>;

    async fn setup(
        path: &Path,
        max_channel_size: u64,
        max_nb_of_infos_in_db: u64,
    ) -> anyhow::Result<(
        Sender<StateRoot, Witness, MockDaSpec>,
        Receiver<StateRoot, Witness, MockDaSpec>,
    )> {
        setup_with_resume(path, max_channel_size, max_nb_of_infos_in_db, None).await
    }

    async fn setup_with_resume(
        path: &Path,
        max_channel_size: u64,
        max_nb_of_infos_in_db: u64,
        latest_proof_final_slot: Option<SlotNumber>,
    ) -> anyhow::Result<(
        Sender<StateRoot, Witness, MockDaSpec>,
        Receiver<StateRoot, Witness, MockDaSpec>,
    )> {
        let (mut sender, receiver) = new_stf_info_channel::<StateRoot, Witness, MockDaSpec>(
            ProofManagerDb::open(path)?,
            NonZero::new(max_channel_size).unwrap(),
            NonZero::new(max_nb_of_infos_in_db).unwrap(),
            latest_proof_final_slot,
        )
        .await?;

        sender
            .startup_notify_about_infos_from_db(&InfiniteHeight)
            .await?;

        Ok((sender, receiver))
    }

    /// Stages the STF info of `height` and writes it as finalized.
    fn write_finalized(sender: &mut Sender<StateRoot, Witness, MockDaSpec>, height: u64) {
        let stf_info = make_stf_info(height);
        sender.stage_stf_info(&stf_info);
        sender
            .write_finalized_stf_infos([stf_info.da_block_header().hash().into()])
            .unwrap();
    }

    fn oldest_slot_number(receiver: &Receiver<StateRoot, Witness, MockDaSpec>) -> u64 {
        (1..=1000)
            .find(|height| receiver.get(SlotNumber::new(*height)).unwrap().is_some())
            .expect("Db is empty")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_start_stop_db() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;

        let channel_size = 10;
        let max_nb_of_infos_in_db = 10;

        // Write some data to the Db.
        {
            let (mut sender, _receiver) =
                setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db).await?;

            for height in 1..=channel_size {
                write_finalized(&mut sender, height);
                sender.notify(SlotNumber::new(height)).await?;
            }
        }

        // Restart the Db and check that we can read the previously written data.
        {
            let (_sender, mut receiver) =
                setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db).await?;

            for i in 1..=channel_size {
                let stf_info = receiver.read_next().await?.unwrap();
                assert_eq!(stf_info.slot_number().get(), i);
            }

            assert_eq!(oldest_slot_number(&receiver), 1);
        }

        // We haven't advanced the cursor above so after restart we will read the same data.
        {
            let (mut sender, mut receiver) =
                setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db).await?;

            for i in 1..=channel_size {
                let stf_info = receiver.read_next().await?.unwrap();
                assert_eq!(stf_info.slot_number().get(), i);
            }

            assert_eq!(oldest_slot_number(&receiver), 1);

            // Write new data.
            receiver
                .next_height_to_receive
                .fetch_add(2, Ordering::SeqCst);

            write_finalized(&mut sender, channel_size + 1);
            sender.notify(SlotNumber::new(channel_size + 1)).await?;

            write_finalized(&mut sender, channel_size + 2);
            sender.notify(SlotNumber::new(channel_size + 2)).await?;

            assert_eq!(oldest_slot_number(&receiver), 2);
        }

        // Now the reads are visible.
        {
            // The previous block advanced `next_height_to_receive` to 3 in
            // memory and persisted writes through slot 12. Resume the channel
            // at slot 3 by anchoring to a "previous proof" with `final_slot=2`.
            let (_sender, mut receiver) = setup_with_resume(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                Some(SlotNumber::new(2)),
            )
            .await?;

            let stf_info = receiver.read_next().await?.unwrap();
            assert_eq!(stf_info.slot_number().get(), 3);
            assert_eq!(oldest_slot_number(&receiver), 2);
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
            let (mut sender, _receiver) = setup_with_resume(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                Some(SlotNumber::new(2)),
            )
            .await?;

            for height in 5..=6 {
                write_finalized(&mut sender, height);
            }
        }

        {
            let (_, mut receiver) = setup_with_resume(
                temp_dir.path(),
                channel_size,
                max_nb_of_infos_in_db,
                Some(SlotNumber::new(2)),
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
    async fn test_only_finalized_fork_is_written() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let (mut sender, receiver) = setup(temp_dir.path(), 10, 100).await?;

        let orphan = make_stf_info_with_hash(1, [0xEE; 32]);
        let canonical = make_stf_info(1);
        let next = make_stf_info(2);
        sender.stage_stf_info(&orphan);
        sender.stage_stf_info(&canonical);
        sender.stage_stf_info(&next);

        // Nothing is written before finalization.
        assert!(receiver.get(SlotNumber::ONE)?.is_none());

        sender.write_finalized_stf_infos([canonical.da_block_header().hash().into()])?;

        let written = receiver.get(SlotNumber::ONE)?.unwrap();
        assert_eq!(get_header_hash(&written), get_header_hash(&canonical));
        assert!(receiver.get(SlotNumber::new(2))?.is_none());
        // The orphan is dropped, the not yet finalized slot 2 stays staged.
        assert_eq!(
            sender.pending.keys().copied().collect::<Vec<DbHash>>(),
            vec![DbHash::from(next.da_block_header().hash())]
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_channel() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        let (mut sender, mut receiver) =
            setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db).await?;

        // Fill the db.
        for height in 1..channel_size {
            write_finalized(&mut sender, height);
            sender.notify(SlotNumber::new(height)).await?;
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

    async fn test_stf_info_drop_sender() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb_of_infos_in_db = 100;

        let (mut sender, mut receiver) =
            setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db).await?;

        for height in 1..3 {
            write_finalized(&mut sender, height);
            sender.notify(SlotNumber::new(height)).await?;
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

        let (mut sender, mut receiver) =
            setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db).await?;

        tokio::spawn(async move {
            // Fill the db.
            for height in 1..=channel_size {
                write_finalized(&mut sender, height);
                sender.notify(SlotNumber::new(height)).await.unwrap();
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

        let (mut sender, mut receiver) =
            setup(temp_dir.path(), channel_size, max_nb_of_infos_in_db).await?;

        // At the begining the db should be empty.
        let fetched_stf_info = receiver.get(SlotNumber::ONE)?;
        assert!(fetched_stf_info.is_none());

        // Insert astf info two times.
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

            let (mut sender, mut receiver) = setup(
                temp_dir.path(),
                test_case.channel_size,
                test_case.max_nb_of_infos_in_db,
            )
            .await?;

            // Fill the db.
            for height in 1..test_case.nb_of_stf_infos {
                write_finalized(&mut sender, height);
                sender.notify(SlotNumber::new(height)).await?;
                receiver.read_next().await?.unwrap();
                receiver
                    .next_height_to_receive
                    .fetch_add(1, Ordering::SeqCst);
            }

            let oldest_height = oldest_slot_number(&receiver);
            assert_eq!(oldest_height, expected_oldest_height);

            // Check if the old STF infos are pruned.
            for height in 1..test_case.nb_of_stf_infos {
                let stf_info: Option<StateTransitionInfo<Vec<u8>, Vec<u8>, MockDaSpec>> =
                    receiver.get(SlotNumber::new(height))?;

                if height < oldest_height {
                    // The old data was deleted from the Db.
                    assert!(stf_info.is_none());
                } else {
                    assert!(stf_info.is_some());
                }
            }
        }
        Ok(())
    }

    async fn assert_stf_in_db(
        rollup_height: u64,
        sender: &mut Sender<StateRoot, Witness, MockDaSpec>,
        receiver: &mut Receiver<StateRoot, Witness, MockDaSpec>,
    ) {
        let original_state_transition_info = make_stf_info(rollup_height);

        write_finalized(sender, rollup_height);
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
        make_stf_info_with_hash(height, [height as u8; 32])
    }

    fn make_stf_info_with_hash(
        height: u64,
        hash: [u8; 32],
    ) -> StateTransitionInfo<Vec<u8>, Vec<u8>, MockDaSpec> {
        StateTransitionInfo::new(StateTransitionWitness {
            initial_state_root: vec![1, 2, 3],
            final_state_root: vec![3, 4, 5],
            da_block_header: MockBlockHeader {
                prev_hash: [0; 32].into(),
                hash: MockHash(hash),
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
