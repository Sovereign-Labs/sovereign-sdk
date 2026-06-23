#![allow(dead_code)]
use std::marker::PhantomData;
use std::num::NonZero;
use std::sync::{Arc, Mutex};

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

/// Shared in-memory `next_height_to_receive` cursor.
///
/// A single mutex serializes all access so concurrent advances (the intake skip-advance and the
/// aggregator advance, running on separate tasks) cannot lose increments. The cursor is never
/// persisted; it is recomputed on startup as `latest_proof_final_slot + 1`.
struct Cursor {
    value: Mutex<SlotNumber>,
}

impl Cursor {
    fn new(value: SlotNumber) -> Self {
        Self {
            value: Mutex::new(value),
        }
    }

    fn get(&self) -> SlotNumber {
        *self.lock()
    }

    fn fetch_add(&self, amount: u64) -> SlotNumber {
        let mut value = self.lock();
        let old_value = *value;
        *value = old_value.saturating_add(amount);
        old_value
    }

    fn fetch_max(&self, other: SlotNumber) {
        let mut value = self.lock();
        *value = (*value).max(other);
    }

    fn store(&self, new_value: SlotNumber) {
        *self.lock() = new_value;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SlotNumber> {
        self.value.lock().expect("cursor lock poisoned")
    }
}

/// Materializes STF infos and sends notifications to the associated [`Receiver`].
pub struct Sender<StateRoot, Witness, Da: DaSpec> {
    /// Height of the next `StateTransitionInfo` that should be received by the [`Receiver`].
    /// This value is synchronized with the receiver end of the channel.
    next_height_to_receive: Arc<Cursor>,

    /// The next height to send to the [`Receiver`]. Used only to avoid sending the same height
    /// notification twice.
    pub next_height_to_send: SlotNumber,

    /// In-memory finalized-visible cutoff: the highest finalized slot the consumer may be notified
    /// about. Replaces the previously-persisted `write_height`; recomputed on startup by
    /// [`Self::startup_notify_about_infos_from_db`] and advanced by [`Self::commit_stf_info`].
    /// [`SlotNumber::GENESIS`] means "nothing finalized-visible yet".
    finalized_visible_height: SlotNumber,

    /// Final slot of the latest accepted aggregated proof known to LedgerDb at startup, if any.
    /// The receive cursor resumes one slot past it so the prover feeds the restored outer
    /// aggregation host a chain that is contiguous with the proof it continues from.
    latest_proof_final_slot: Option<SlotNumber>,

    /// Whether startup may skip a missing local STF-info prefix.
    ///
    /// This is only sound when no previous outer proof is being restored. If a previous outer
    /// proof exists, the aggregation circuit requires the next inner proof to start exactly at
    /// `latest_proof_final_slot + 1`.
    allow_missing_local_stf_prefix: bool,

    // Max number of entries we will keep in the Db, older data will be pruned.
    max_nb_of_infos_in_db: NonZero<u64>,

    /// The notification channel carries `(slot, canonical DA hash)` — the hash, resolved on the
    /// producer side, lets the [`Receiver`] read the finalized fork directly without consulting
    /// LedgerDb. It does not contain the STF-info data itself, only the key.
    ///
    /// ## Note
    /// This channel enforces back-pressure: when full, the rollup halts and waits for the
    /// [`Receiver`] to catch up. Its size must not exceed [`Sender::max_nb_of_infos_in_db`].
    notifier: mpsc::Sender<(SlotNumber, DbHash)>,

    /// The proof manager database storing the STF-info queue.
    proof_manager_db: ProofManagerDb,

    _phantom: PhantomData<(StateRoot, Witness, Da)>,
}

impl<
        StateRoot: DeserializeOwned + Serialize,
        Witness: DeserializeOwned + Serialize,
        Da: DaSpec,
    > Sender<StateRoot, Witness, Da>
{
    /// This method is only called when starting up the stf info manager. It ensures the in-memory
    /// state is correctly recomputed from the ledger view and synchronized with the receiver.
    ///
    /// `ledger_head` and `latest_finalized_slot_number` reconcile the proof-manager queue with the
    /// ledger state, ensuring the proof manager never exposes data beyond the latest finalized
    /// slot while still preserving staged future rows. `resolve_hash` maps a slot to the ledger's
    /// canonical DA hash so the finalized fork is selected without coupling the receiver to
    /// LedgerDb.
    pub(crate) async fn startup_notify_about_infos_from_db(
        &mut self,
        ledger_head: SlotNumber,
        latest_finalized_slot_number: SlotNumber,
        max_provable_slot_number: &dyn ProvableHeightTracker,
        resolve_hash: impl Fn(SlotNumber) -> anyhow::Result<Option<DbHash>>,
    ) -> anyhow::Result<()> {
        // Recompute the finalized-visible cutoff from the ledger view (no persisted metadata).
        self.finalized_visible_height = self.proof_manager_db.recompute_visible_state(
            ledger_head,
            latest_finalized_slot_number,
            &resolve_hash,
        )?;

        // The receive cursor is in-memory only; recompute it from the latest accepted proof.
        let next_height_to_receive = load_next_height_to_receive(self.latest_proof_final_slot);
        self.next_height_to_receive.store(next_height_to_receive);
        self.next_height_to_send = next_height_to_receive;

        // Capacity invariant: the consumer must not have fallen so far behind the finalized cutoff
        // that more than `max_nb_of_infos_in_db` visible infos are pending.
        if next_height_to_receive <= self.finalized_visible_height {
            let unread_visible_infos = self
                .finalized_visible_height
                .get()
                .saturating_sub(next_height_to_receive.get());
            assert!(
                unread_visible_infos <= self.max_nb_of_infos_in_db.get(),
                "Too many STF infos in the db: {} vs max allowed {} (next_to_receive={} finalized_visible={})",
                unread_visible_infos,
                self.max_nb_of_infos_in_db,
                next_height_to_receive,
                self.finalized_visible_height,
            );
        }

        // Notify about visible pending STF info; the receiver drains each transition from its
        // current cursor up to this height.
        let max_provable_slot_number = max_provable_slot_number.max_provable_slot_number();
        self.notify(max_provable_slot_number, &resolve_hash).await?;

        Ok(())
    }

    /// Get the next height to receive as a `SlotNumber`.
    pub fn next_height_to_receive(&self) -> SlotNumber {
        self.next_height_to_receive.get()
    }

    /// Increment next height to receive by one, returning the previous value. Test-only.
    #[cfg(test)]
    pub fn inc_next_height_to_receive(&self) -> SlotNumber {
        self.next_height_to_receive.fetch_add(1)
    }
}

/// Receives notifications from the associated [`Sender`] and reads STF info data from the db.
pub struct Receiver<StateRoot, Witness, Da: DaSpec> {
    /// Height of the next `StateTransitionInfo` that is expected to be processed by the `Receiver`
    next_height_to_receive: Arc<Cursor>,
    proof_manager_db: ProofManagerDb,
    receiver: mpsc::Receiver<(SlotNumber, DbHash)>,
    _phantom: PhantomData<(StateRoot, Witness, Da)>,
}

/// Creates a new [`Sender`] and [`Receiver`] channel.
///
/// The STF-info queue is retained across Db restarts; the cursors are not (they are recomputed on
/// startup).
/// - The sender blocks if the channel reaches `max_channel_size` of STF infos.
/// - If the number of entries in the Db exceeds `max_nb_of_infos_in_db`, the oldest data is pruned.
///
/// The channel can only be created if `max_channel_size <= max_nb_of_infos_in_db`.
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
    new_stf_info_channel_inner(
        proof_manager_db,
        max_channel_size,
        max_nb_of_infos_in_db,
        latest_proof_final_slot,
        latest_proof_final_slot.is_none(),
    )
}

#[allow(clippy::type_complexity)]
pub(crate) fn new_stf_info_channel_for_runner<StateRoot, Witness, Da: DaSpec>(
    proof_manager_db: ProofManagerDb,
    max_channel_size: NonZero<u64>,
    max_nb_of_infos_in_db: NonZero<u64>,
    latest_proof_final_slot: Option<SlotNumber>,
    start_fresh_outer_proof_on_resync: bool,
) -> anyhow::Result<(
    Sender<StateRoot, Witness, Da>,
    Receiver<StateRoot, Witness, Da>,
)> {
    // The in-memory receive cursor can re-prove an already-posted window after a crash (it resumes
    // from the latest *accepted* proof, which may lag a just-posted one). That is only sound when
    // a fresh outer proof replaces the previous one; otherwise the re-posted proof would form a
    // non-contiguous chain. Require the flag whenever we resume from an existing outer proof.
    if latest_proof_final_slot.is_some() && !start_fresh_outer_proof_on_resync {
        bail!(
            "The zk proving pipeline requires `--start-fresh-outer-proof-on-resync` when resuming \
             from an existing outer proof (latest_proof_final_slot={latest_proof_final_slot:?}). \
             The in-memory receive cursor may re-prove an already-posted window after a crash; \
             without a fresh outer proof that would produce a non-contiguous proof chain."
        );
    }

    let allow_missing_local_stf_prefix =
        latest_proof_final_slot.is_none() || start_fresh_outer_proof_on_resync;
    new_stf_info_channel_inner(
        proof_manager_db,
        max_channel_size,
        max_nb_of_infos_in_db,
        latest_proof_final_slot,
        allow_missing_local_stf_prefix,
    )
}

#[allow(clippy::type_complexity)]
fn new_stf_info_channel_inner<StateRoot, Witness, Da: DaSpec>(
    proof_manager_db: ProofManagerDb,
    max_channel_size: NonZero<u64>,
    max_nb_of_infos_in_db: NonZero<u64>,
    latest_proof_final_slot: Option<SlotNumber>,
    allow_missing_local_stf_prefix: bool,
) -> anyhow::Result<(
    Sender<StateRoot, Witness, Da>,
    Receiver<StateRoot, Witness, Da>,
)> {
    assert!(
        max_channel_size <= max_nb_of_infos_in_db,
        "Channel size should be smaller than the max number of STFInfos in the db"
    );

    let (notifier, receiver) =
        tokio::sync::mpsc::channel::<(SlotNumber, DbHash)>(max_channel_size.get().try_into()?);

    // Resume STF-info processing from the last aggregated proof verified on-chain: that proof's
    // `final_slot` is the last slot we know is committed, so the prover picks up at `final_slot +
    // 1`. If no such proof exists yet (fresh node), start from genesis.
    let next_height_to_receive = load_next_height_to_receive(latest_proof_final_slot);

    let cursor = Arc::new(Cursor::new(next_height_to_receive));

    let sender = Sender {
        max_nb_of_infos_in_db,
        next_height_to_receive: cursor.clone(),
        next_height_to_send: next_height_to_receive,
        finalized_visible_height: SlotNumber::GENESIS,
        latest_proof_final_slot,
        allow_missing_local_stf_prefix,
        notifier,
        proof_manager_db: proof_manager_db.clone(),
        _phantom: PhantomData,
    };

    let receiver = Receiver {
        next_height_to_receive: cursor,
        proof_manager_db,
        receiver,
        _phantom: PhantomData,
    };

    Ok((sender, receiver))
}

/// Compute the in-memory receive cursor resume point.
///
/// If a predecessor aggregated proof exists, resume exactly one slot past its final slot: the
/// restored outer aggregation host requires a contiguous inner-proof chain starting there. If no
/// proof exists yet (fresh node, or optimistic mode), resume from genesis. The cursor is never
/// persisted — on restart we re-prove from this point, which is sound because the node always runs
/// with `--start-fresh-outer-proof-on-resync`.
fn load_next_height_to_receive(latest_proof_final_slot: Option<SlotNumber>) -> SlotNumber {
    match latest_proof_final_slot {
        Some(final_slot) => final_slot.saturating_add(1),
        None => SlotNumber::ONE,
    }
}

impl<StateRoot, Witness, Da: DaSpec> Sender<StateRoot, Witness, Da>
where
    StateRoot: Serialize + DeserializeOwned,
    Witness: Serialize + DeserializeOwned,
{
    /// Sends the stf update notifications and updates the last submitted height.
    ///
    /// `resolve_hash` supplies the canonical DA hash for each notified (finalized) slot; it is
    /// stamped onto the notification so the consumer reads the finalized fork directly.
    pub async fn notify(
        &mut self,
        max_provable_slot_number: SlotNumber,
        resolve_hash: impl Fn(SlotNumber) -> anyhow::Result<Option<DbHash>>,
    ) -> anyhow::Result<()> {
        let finalized_visible_height = self.finalized_visible_height;
        if finalized_visible_height == SlotNumber::GENESIS {
            // Nothing finalized-visible yet, so there is nothing to notify.
            return Ok(());
        }

        self.realign_notifier_cursor_to_available_stf_info(finalized_visible_height)
            .await?;
        let next_height_to_send = self.next_height_to_send;

        // Never notify for a height that is not finalized and visible, even if later STF rows have
        // already been staged.
        let height_to_notify = std::cmp::min(max_provable_slot_number, finalized_visible_height);
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
            // Stamp the canonical DA hash so the consumer reads the finalized fork without
            // resolving it itself (and without coupling the receiver to LedgerDb).
            let hash = resolve_hash(height)?.ok_or_else(|| {
                anyhow!("missing canonical DA hash for finalized slot {height}; cannot notify")
            })?;
            tracing::trace!(%height, "Requesting for proving/attesting");
            self.notifier.send((height, hash)).await?;
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

    /// Stage STF info data in ProofManagerDb immediately, keyed by `(slot, DA block hash)`.
    ///
    /// # Two-phase commit pattern
    ///
    /// Because ProofManagerDb and LedgerDb are separate databases, they cannot be committed
    /// atomically. To maintain consistency:
    ///
    /// 1. **Stage** (this method): writes STF info data to ProofManagerDb before the ledger commit.
    /// 2. **Commit** ([`Self::commit_stf_info`]): after the ledger commit succeeds, advances the
    ///    in-memory finalized-visible cutoff and prunes.
    ///
    /// If a crash occurs between staging and committing,
    /// [`ProofManagerDb::recompute_visible_state`] re-exposes only the contiguous portion that is
    /// finalized according to LedgerDb and drops any staged rows above the ledger head.
    pub async fn stage_stf_info(
        &self,
        stf_info: &StateTransitionInfo<StateRoot, Witness, Da>,
    ) -> anyhow::Result<()> {
        let encoded_stf_info: Vec<u8> = bincode::serialize(stf_info).unwrap();
        let stored_stf_info = StoredStfInfo {
            data: encoded_stf_info,
        };
        let write_rollup_height = stf_info.slot_number();
        // Key by the DA block hash so competing forks at the same slot are distinct rows. This is
        // the same hash the ledger stores in `StoredSlot.hash` (the canonical-fork selector).
        let da_block_hash: DbHash = stf_info.da_block_header().hash().into();
        let db = self.proof_manager_db.clone();

        tokio::task::spawn_blocking(move || {
            db.put_stf_info(write_rollup_height, da_block_hash, &stored_stf_info)
        })
        .await
        .map_err(|e| anyhow!("ProofManagerDb write task failed: {e}"))??;

        tracing::trace!(%write_rollup_height, "Done staging stf_info data");
        Ok(())
    }

    /// Commit STF info after ledger finality advances: advance the in-memory finalized-visible
    /// cutoff and prune below the retention window.
    pub async fn commit_stf_info(&mut self, write_rollup_height: SlotNumber) -> anyhow::Result<()> {
        self.finalized_visible_height = write_rollup_height.max(self.finalized_visible_height);
        let finalized_visible_height = self.finalized_visible_height;
        let next_rollup_height_to_receive = self.next_height_to_receive();
        let max_nb_of_infos_in_db = self.max_nb_of_infos_in_db;
        let db = self.proof_manager_db.clone();

        tokio::task::spawn_blocking(move || {
            db.prune(
                finalized_visible_height,
                next_rollup_height_to_receive,
                max_nb_of_infos_in_db,
            )
        })
        .await
        .map_err(|e| anyhow!("ProofManagerDb write task failed: {e}"))??;
        Ok(())
    }

    fn get_oldest_slot_number(&self) -> anyhow::Result<SlotNumber> {
        Ok(self
            .proof_manager_db
            .oldest_present_slot()?
            .unwrap_or(SlotNumber::ONE))
    }

    async fn realign_notifier_cursor_to_available_stf_info(
        &mut self,
        finalized_visible_height: SlotNumber,
    ) -> anyhow::Result<()> {
        if self.next_height_to_send > finalized_visible_height {
            return Ok(());
        }

        let resume_cursor = self.next_height_to_send;
        for height in resume_cursor.range_inclusive(finalized_visible_height) {
            if self.proof_manager_db.has_row_at_slot(height)? {
                if height > resume_cursor {
                    if !self.allow_missing_local_stf_prefix {
                        bail!(
                            "Cannot realign STF-info notifier cursor from {resume_cursor} to first local slot {height}: \
                             latest_proof_final_slot={:?} requires recursive proof continuity from slot {resume_cursor}. \
                             Start with a fresh outer proof before skipping locally-missing STF infos",
                            self.latest_proof_final_slot
                        );
                    }

                    // The consumer's resume cursor (derived from the last durably-observed
                    // proof/attestation) sits before the first STF info this node actually has
                    // locally. This is the expected shape when a node ran without a proof pipeline
                    // and now starts one (e.g. a former replica taking over leadership). The slots
                    // in the gap will never be proven by this node.
                    tracing::warn!(
                        resume_cursor = %resume_cursor,
                        first_local_slot = %height,
                        %finalized_visible_height,
                        "Realigning STF-info notifier cursor: resume position sits before the first locally-materialized STF info. \
                         Slots in [{resume_cursor}, {height}) are not present locally and will be skipped. \
                         This usually means the node previously ran without the proof pipeline (e.g. as a replica)."
                    );
                    self.next_height_to_send = height;
                    self.next_height_to_receive.fetch_max(height);
                }
                return Ok(());
            }
        }

        bail!(
            "The `stf-info-manager` cutoff says STF infos exist through slot {finalized_visible_height}, \
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
    /// This method blocks if the channel is empty (the producer is slower than the consumer).
    /// Returns `Ok(None)` if the stf info for the given height has already been read and processed.
    /// Returns `anyhow::Error` if the channel is closed.
    pub async fn read_next(
        &mut self,
    ) -> anyhow::Result<Option<StateTransitionInfo<StateRoot, Witness, Da>>> {
        if let Some((slot_number, da_block_hash)) = self.receiver.recv().await {
            tracing::trace!(%slot_number, "Read next stf info");
            let next_height_to_receive = self.next_height_to_receive();
            tracing::trace!(%next_height_to_receive, %slot_number, "Next height to receive");

            // Ensure the received height is at or beyond the next expected height so we never
            // process the same height twice.
            if slot_number >= next_height_to_receive {
                let stf_info = self.get(slot_number, da_block_hash)?.unwrap_or_else(|| {
                    panic!("The `stf-info-manager` sender notified that the stf height {slot_number} (canonical fork) is available but the transition is missing from the proof manager DB.
                    Please ensure that the `stf-info-manager` only notifies for heights up to the finalized-visible cutoff. This is a bug. Please report it")
                });

                return Ok(Some(stf_info));
            }

            return Ok(None);
        }

        bail!("Channel closed. Impossible to read next stf info")
    }

    /// Gets [`StateTransitionInfo`] for the corresponding `(slot, DA block hash)`.
    fn get(
        &self,
        slot_number: SlotNumber,
        da_block_hash: DbHash,
    ) -> anyhow::Result<Option<StateTransitionInfo<StateRoot, Witness, Da>>> {
        let maybe_stored_stf_info = self
            .proof_manager_db
            .get_stf_info(slot_number, da_block_hash)?;

        if let Some(stored_stf_info) = maybe_stored_stf_info {
            Ok(Some(bincode::deserialize(&stored_stf_info.data[..])?))
        } else {
            Ok(None)
        }
    }

    /// Get the next height to receive as a `SlotNumber`.
    pub fn next_height_to_receive(&self) -> SlotNumber {
        self.next_height_to_receive.get()
    }

    /// Increment next height to receive by the requested amount (in-memory only), returning the
    /// previous value.
    pub fn inc_next_height_to_receive_by(&self, amount: u64) -> SlotNumber {
        self.cursor_handle().inc_next_height_to_receive_by(amount)
    }

    /// Returns a cloneable handle that can advance `next_height_to_receive` from a different task
    /// without requiring access to the full [`Receiver`]. Used by the proof-aggregation pipeline
    /// to keep the cursor advance co-located with the actual proof publication, while the
    /// channel-draining `read_next` still happens on the intake task.
    pub fn cursor_handle(&self) -> CursorHandle {
        CursorHandle {
            next_height_to_receive: self.next_height_to_receive.clone(),
        }
    }
}

/// A clonable handle over the [`Receiver`]'s `next_height_to_receive` cursor.
///
/// The cursor doubles as the prune cutoff for materialized STF infos, so it MUST only be advanced
/// over slots that the consumer has finished with — either because their proof has been durably
/// published, or because they have been intentionally abandoned (e.g. slots inside a resync window
/// that the prover has decided to skip). Holding this handle in a non-receiver task lets the
/// consumer do that advance without giving the task the rest of the [`Receiver`].
#[derive(Clone)]
pub struct CursorHandle {
    next_height_to_receive: Arc<Cursor>,
}

impl CursorHandle {
    /// Increment next height to receive by the requested amount (in-memory only), returning the
    /// previous value.
    ///
    /// The cursor is never persisted: on restart it is recomputed as `latest_proof_final_slot + 1`.
    /// Any re-proof of an already-posted window is tolerated because the node always runs with
    /// `--start-fresh-outer-proof-on-resync`, which replaces the previous outer proof.
    pub fn inc_next_height_to_receive_by(&self, amount: u64) -> SlotNumber {
        self.next_height_to_receive.fetch_add(amount)
    }

    /// Current in-memory next height to receive.
    pub fn next_height_to_receive(&self) -> SlotNumber {
        self.next_height_to_receive.get()
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

    /// Canonical DA hash for a slot, matching the hash `make_stf_info` stamps on its header. Used
    /// as the producer-side resolver in tests.
    fn canonical_hash_for(slot: SlotNumber) -> DbHash {
        [slot.get() as u8; 32]
    }

    fn resolver() -> impl Fn(SlotNumber) -> anyhow::Result<Option<DbHash>> {
        |slot| Ok(Some(canonical_hash_for(slot)))
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
        setup_with_resume_and_missing_prefix_policy(
            path,
            max_channel_size,
            max_nb_of_infos_in_db,
            latest_proof_final_slot,
            latest_proof_final_slot.is_none(),
        )
    }

    #[allow(clippy::type_complexity)]
    fn setup_with_resume_and_missing_prefix_policy(
        path: &Path,
        max_channel_size: u64,
        max_nb_of_infos_in_db: u64,
        latest_proof_final_slot: Option<SlotNumber>,
        allow_missing_local_stf_prefix: bool,
    ) -> anyhow::Result<(
        ProofManagerDb,
        Sender<StateRoot, Witness, MockDaSpec>,
        Receiver<StateRoot, Witness, MockDaSpec>,
    )> {
        let proof_manager_db = ProofManagerDb::open(path)?;

        let (sender, receiver) = new_stf_info_channel_inner::<StateRoot, Witness, MockDaSpec>(
            proof_manager_db.clone(),
            NonZero::new(max_channel_size).unwrap(),
            NonZero::new(max_nb_of_infos_in_db).unwrap(),
            latest_proof_final_slot,
            allow_missing_local_stf_prefix,
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
            .startup_notify_about_infos_from_db(
                ledger_head,
                ledger_head,
                &InfiniteHeight,
                resolver(),
            )
            .await?;

        Ok((proof_manager_db, sender, receiver))
    }

    /// Stage + commit + notify a single canonical slot.
    async fn produce(
        sender: &mut Sender<StateRoot, Witness, MockDaSpec>,
        height: u64,
    ) -> anyhow::Result<()> {
        let stf_info = make_stf_info(height);
        sender.stage_stf_info(&stf_info).await?;
        sender.commit_stf_info(stf_info.slot_number()).await?;
        sender.notify(stf_info.slot_number(), resolver()).await?;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stf_info_channel_roundtrip() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb = 100;

        let (_db, mut sender, mut receiver) = setup_with_startup(
            temp_dir.path(),
            channel_size,
            max_nb,
            SlotNumber::new(channel_size),
        )
        .await?;

        for height in 1..channel_size {
            produce(&mut sender, height).await?;
        }

        for height in 1..channel_size {
            let stf_info = receiver.read_next().await?.unwrap();
            receiver.inc_next_height_to_receive_by(1);
            assert_eq!(stf_info.slot_number().get(), height);
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_read_selects_canonical_fork() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let (db, mut sender, mut receiver) =
            setup_with_startup(temp_dir.path(), 10, 100, SlotNumber::new(3)).await?;

        // Stage an orphan fork at slot 1 (different hash), then the canonical slot 1.
        let orphan = make_stf_info_with_hash(1, [0xEE; 32]);
        sender.stage_stf_info(&orphan).await?;
        produce(&mut sender, 1).await?;

        // Both rows exist in the DB.
        assert!(db.get_stf_info(SlotNumber::ONE, [0xEE; 32])?.is_some());
        assert!(db
            .get_stf_info(SlotNumber::ONE, canonical_hash_for(SlotNumber::ONE))?
            .is_some());

        // The consumer is notified with the canonical hash and reads the canonical row.
        let received = receiver.read_next().await?.unwrap();
        assert_eq!(received.slot_number(), SlotNumber::ONE);
        assert_eq!(received.da_block_header().hash, MockHash([1u8; 32]));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_startup_recompute_and_reread_uncommitted() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb = 10;

        {
            let (_db, mut sender, _receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb,
                SlotNumber::new(channel_size),
            )
            .await?;
            for height in 1..=channel_size {
                produce(&mut sender, height).await?;
            }
        }

        // Restart: cursor is recomputed (no proof => genesis), so we re-read from slot 1.
        {
            let (_db, _sender, mut receiver) = setup_with_startup(
                temp_dir.path(),
                channel_size,
                max_nb,
                SlotNumber::new(channel_size),
            )
            .await?;
            for i in 1..=channel_size {
                let stf_info = receiver.read_next().await?.unwrap();
                assert_eq!(stf_info.slot_number().get(), i);
            }
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reprove_resumes_from_latest_proof_after_restart() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;

        // First run: a previous outer proof reached slot 100; stage slot 101.
        {
            let (_db, mut sender, _receiver) = setup_with_resume_and_missing_prefix_policy(
                temp_dir.path(),
                10,
                100,
                Some(SlotNumber::new(100)),
                true,
            )?;
            assert_eq!(sender.next_height_to_receive(), SlotNumber::new(101));
            produce(&mut sender, 101).await?;
        }

        // Restart with the latest accepted proof still at 100 (a crash lost the just-posted one):
        // the in-memory cursor resumes at 101 and re-proves it.
        {
            let (_db, mut sender, mut receiver) = setup_with_resume_and_missing_prefix_policy(
                temp_dir.path(),
                10,
                100,
                Some(SlotNumber::new(100)),
                true,
            )?;
            sender
                .startup_notify_about_infos_from_db(
                    SlotNumber::new(101),
                    SlotNumber::new(101),
                    &InfiniteHeight,
                    resolver(),
                )
                .await?;
            assert_eq!(receiver.next_height_to_receive(), SlotNumber::new(101));
            let received = receiver.read_next().await?.unwrap();
            assert_eq!(received.slot_number(), SlotNumber::new(101));
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_empty_db_notify_is_noop() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let (_db, mut sender, _receiver) =
            setup_with_startup(temp_dir.path(), 10, 100, SlotNumber::GENESIS).await?;

        // No STF rows: finalized-visible cutoff is genesis, so notify sends nothing.
        sender.notify(SlotNumber::new(5), resolver()).await?;
        assert_eq!(sender.next_height_to_send, SlotNumber::ONE);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_pruning_advances_oldest() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 15;
        let max_nb = 15;
        let nb_infos = 30;

        let (_db, mut sender, mut receiver) = setup_with_startup(
            temp_dir.path(),
            channel_size,
            max_nb,
            SlotNumber::new(nb_infos),
        )
        .await?;

        for height in 1..nb_infos {
            produce(&mut sender, height).await?;
            receiver.read_next().await?.unwrap();
            receiver.inc_next_height_to_receive_by(1);
        }

        // With finalized_visible ~= nb_infos-1 and max_nb retained, the oldest surviving slot is
        // (nb_infos-1) - max_nb.
        let oldest = sender.get_oldest_slot_number()?;
        assert_eq!(oldest.get(), (nb_infos - 1) - max_nb);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_realign_missing_prefix_without_previous_proof() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb = 10;

        // Fresh node (no proof); the local node only has STF infos starting at 5.
        {
            let (_db, mut sender, _receiver) =
                setup_with_resume(temp_dir.path(), channel_size, max_nb, None)?;
            for height in 5..=6 {
                let stf_info = make_stf_info(height);
                sender.stage_stf_info(&stf_info).await?;
                sender.commit_stf_info(stf_info.slot_number()).await?;
            }
        }

        {
            let (_db, mut sender, mut receiver) =
                setup_with_resume(temp_dir.path(), channel_size, max_nb, None)?;
            sender
                .startup_notify_about_infos_from_db(
                    SlotNumber::new(6),
                    SlotNumber::new(6),
                    &InfiniteHeight,
                    resolver(),
                )
                .await?;

            assert_eq!(receiver.read_next().await?.unwrap().slot_number().get(), 5);
            assert_eq!(receiver.read_next().await?.unwrap().slot_number().get(), 6);
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_realign_rejects_missing_prefix_with_previous_proof() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb = 10;

        // Previous proof at slot 2, but the local node only has STF infos starting at 5, and the
        // missing-prefix policy is disallowed (flag off).
        {
            let (_db, mut sender, _receiver) = setup_with_resume_and_missing_prefix_policy(
                temp_dir.path(),
                channel_size,
                max_nb,
                Some(SlotNumber::new(2)),
                false,
            )?;
            for height in 5..=6 {
                let stf_info = make_stf_info(height);
                sender.stage_stf_info(&stf_info).await?;
                sender.commit_stf_info(stf_info.slot_number()).await?;
            }
        }

        {
            let (_db, mut sender, _receiver) = setup_with_resume_and_missing_prefix_policy(
                temp_dir.path(),
                channel_size,
                max_nb,
                Some(SlotNumber::new(2)),
                false,
            )?;
            let err = sender
                .startup_notify_about_infos_from_db(
                    SlotNumber::new(6),
                    SlotNumber::new(6),
                    &InfiniteHeight,
                    resolver(),
                )
                .await
                .expect_err("missing local prefix must be rejected when restoring an outer proof");
            assert!(
                err.to_string()
                    .contains("requires recursive proof continuity"),
                "unexpected error: {err:?}"
            );
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_realign_missing_prefix_with_fresh_outer_proof() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let channel_size = 10;
        let max_nb = 10;

        // Previous proof at slot 2, local infos start at 5, but fresh-outer policy allows the gap.
        {
            let (_db, mut sender, _receiver) = setup_with_resume_and_missing_prefix_policy(
                temp_dir.path(),
                channel_size,
                max_nb,
                Some(SlotNumber::new(2)),
                true,
            )?;
            for height in 5..=6 {
                let stf_info = make_stf_info(height);
                sender.stage_stf_info(&stf_info).await?;
                sender.commit_stf_info(stf_info.slot_number()).await?;
            }
        }

        {
            let (_db, mut sender, mut receiver) = setup_with_resume_and_missing_prefix_policy(
                temp_dir.path(),
                channel_size,
                max_nb,
                Some(SlotNumber::new(2)),
                true,
            )?;
            sender
                .startup_notify_about_infos_from_db(
                    SlotNumber::new(6),
                    SlotNumber::new(6),
                    &InfiniteHeight,
                    resolver(),
                )
                .await?;
            assert_eq!(receiver.read_next().await?.unwrap().slot_number().get(), 5);
            assert_eq!(receiver.read_next().await?.unwrap().slot_number().get(), 6);
        }
        Ok(())
    }

    #[test]
    fn test_runner_channel_requires_flag_when_resuming_from_proof() {
        let temp_dir = tempfile::tempdir().unwrap();
        let proof_manager_db = ProofManagerDb::open(temp_dir.path()).unwrap();

        let result = new_stf_info_channel_for_runner::<StateRoot, Witness, MockDaSpec>(
            proof_manager_db,
            NonZero::new(10).unwrap(),
            NonZero::new(100).unwrap(),
            Some(SlotNumber::new(5)),
            false,
        );
        let err = match result {
            Ok(_) => panic!("must require the resync flag when resuming from an outer proof"),
            Err(e) => e,
        };
        assert!(
            err.to_string()
                .contains("start-fresh-outer-proof-on-resync"),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn test_load_next_height_to_receive() {
        assert_eq!(load_next_height_to_receive(None), SlotNumber::ONE);
        assert_eq!(
            load_next_height_to_receive(Some(SlotNumber::new(100))),
            SlotNumber::new(101)
        );
    }

    #[test]
    fn test_concurrent_cursor_advances_lose_no_increments() {
        let temp_dir = tempfile::tempdir().unwrap();
        let (_db, _sender, receiver) = setup(temp_dir.path(), 10, 100).unwrap();

        let start = receiver.next_height_to_receive().get();
        const ITERS: u64 = 2_000;

        let a = receiver.cursor_handle();
        let b = receiver.cursor_handle();
        let ta = std::thread::spawn(move || {
            for _ in 0..ITERS {
                a.inc_next_height_to_receive_by(1);
            }
        });
        let tb = std::thread::spawn(move || {
            for _ in 0..ITERS {
                b.inc_next_height_to_receive_by(1);
            }
        });
        ta.join().unwrap();
        tb.join().unwrap();

        assert_eq!(
            receiver.next_height_to_receive().get(),
            start + 2 * ITERS,
            "concurrent cursor advances must not lose any increment"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_drop_sender_closes_channel() -> anyhow::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let (_db, mut sender, mut receiver) =
            setup_with_startup(temp_dir.path(), 10, 100, SlotNumber::new(10)).await?;

        for height in 1..3 {
            produce(&mut sender, height).await?;
        }
        drop(sender);

        for height in 1..3 {
            let stf_info = receiver.read_next().await?.unwrap();
            receiver.inc_next_height_to_receive_by(1);
            assert_eq!(stf_info.slot_number().get(), height);
        }
        assert!(receiver.read_next().await.is_err());
        Ok(())
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
}
