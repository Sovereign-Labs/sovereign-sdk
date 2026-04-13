use super::batch_size_tracker::BatchSizeTracker;
use crate::preferred::block_executor::RollupBlockExecutor;
use crate::preferred::cache_warm_up_executor::CacheWarmUpExecutor;
use crate::preferred::db::BatchToStore;
use crate::preferred::db::SequencerRole;
use crate::preferred::executor_events::ExecutorEventsSender;
use crate::preferred::rate_limiter::IpAndCredentialId;
use crate::preferred::rate_limiter::ResourceLimitExceededError;
use crate::preferred::rate_limiter::SovRateLimiter;
use crate::preferred::replica::event_handler::ReplicaError;
use crate::preferred::replica::event_receiver::EventReceiverStartNotifier;
use crate::preferred::update_state::SequenceNumberMismatchError;
use crate::preferred::AcceptedTx;
use crate::preferred::BatchCreationError;
use crate::preferred::Confirmation;
use crate::preferred::DbEvent;
use crate::preferred::PreferredProofToReplay;
use crate::preferred::PreferredSeqOperation;
use crate::preferred::RollupBlockExecutorConfig;
use crate::preferred::TxResultWriter;
use crate::PreferredProofDataBytes;
use crate::SerializedProofWithDetailsBytes;
use crate::{SequencerNotReadyDetails, TxHash};
pub(crate) use inner::*;
use sov_blob_sender::BlobInternalId;
use sov_blob_storage::SequenceNumber;
use sov_full_node_configs::sequencer::{PreferredSequencerConfig, SequencerConfig};
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::GasArray;
use sov_modules_api::GasSpec;
use sov_modules_api::VersionReader;
use sov_modules_api::{FullyBakedTx, Runtime, Spec, StateUpdateInfo};
use sov_state::Storage;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicUsize};
use std::sync::Arc;
pub(crate) use sync_state::*;
use tokio::sync::broadcast;
use tokio::sync::{mpsc, oneshot, watch};
pub(crate) use updator::*;

mod conditions_table;
mod inner;
mod sync_state;
mod updator;

const CHANNEL_SIZE: usize = 128;

pub(super) enum Message<S: Spec, Rt: Runtime<S>> {
    NextSequenceNumber {
        resp: oneshot::Sender<SequenceNumber>,
        reason: &'static str,
    },
    FetchProofsAndCompletedBatches {
        resp: oneshot::Sender<FetchProofsAndCompletedBatches>,
        next_sequence_number: u64,
        reason: &'static str,
    },
    SequencerConditions {
        resp: oneshot::Sender<PreferredSeqOperation<S, Rt>>,
        info: StateUpdateInfo<S::Storage>,
        next_sequence_number_according_to_node: u64,
        reason: &'static str,
    },
    CheckReadiness {
        resp: oneshot::Sender<Result<(), SequencerNotReadyDetails>>,
        max_concurrent_blobs: usize,
        height_to_stop_at: Option<RollupHeight>,
        reason: &'static str,
    },

    AcceptTx {
        resp: oneshot::Sender<AcceptTxRet<S, Rt>>,
        baked_tx: FullyBakedTx,
        tx_hash: TxHash,
        original_tx_queue_id: u64,
        ip_and_credential: IpAndCredentialId<S::Address>,
        reason: &'static str,
    },

    FinalCatchup {
        resp: oneshot::Sender<Result<ProcessFinalCatchupData, SequenceNumberMismatchError>>,
        info: StateUpdateInfo<S::Storage>,
        db_event_subscription: mpsc::Receiver<DbEvent>,
        executor: Box<RollupBlockExecutor<S, Rt>>,
        node_state_root: <S::Storage as Storage>::Root,
        data: ProcessFinalCatchupData,
        reason: &'static str,
    },
    PruneSequencerDb {
        reason: &'static str,
    },
    ForceOverwriteStateForRecovery {
        info: StateUpdateInfo<S::Storage>,
        reason: &'static str,
    },
    WaitNodeResync {
        info: StateUpdateInfo<S::Storage>,
        distance: u64,
        reason: &'static str,
    },
    #[cfg(feature = "test-utils")]
    ForceCloseCurrentBatch {
        reason: &'static str,
        result_sender: oneshot::Sender<bool>,
    },
    ProofBlob {
        blob_id: BlobInternalId,
        data: SerializedProofWithDetailsBytes,
        reason: &'static str,
    },
    TriggerBatchProduction {
        reason: &'static str,
    },
    SimpleStateUpdate {
        info: StateUpdateInfo<S::Storage>,
    },
    ReplicaBatchStartMsg {
        resp: oneshot::Sender<Result<(), ReplicaError<S>>>,
        batch_from_master: BatchToStore,
        reason: &'static str,
    },
    ReplicaNewTx {
        resp: oneshot::Sender<Result<(), ReplicaError<S>>>,
        seq_nr_from_master: u64,
        tx_hash: TxHash,
        baked_tx: FullyBakedTx,
        reason: &'static str,
    },
    ReplicaCloseCurrentBatch {
        resp: oneshot::Sender<Result<(), ReplicaError<S>>>,
        batch_from_master: BatchToStore,
        reason: &'static str,
    },
    ReplicaNewProof {
        resp: oneshot::Sender<Result<(), ReplicaError<S>>>,
        sequence_number: u64,
        proof_bytes: PreferredProofDataBytes,
        reason: &'static str,
    },
    GetSequencerRole {
        resp: oneshot::Sender<SequencerRole>,
        reason: &'static str,
    },
}

impl<S: Spec, Rt: Runtime<S>> Message<S, Rt> {
    fn priority(&self, runtime: &Rt) -> u64 {
        // Computes the priority of the message. Note that the implementation of the queue assumes that transactions never have higher priority than internal
        // sequencer messages - if we change this assumption, we'll need to update the implementation of sequencer state updater to handle this.
        match self {
            Message::AcceptTx { baked_tx, .. } => {
                let priority = runtime.get_transaction_priority(baked_tx);
                priority as u64
            }
            _ => u64::MAX,
        }
    }
}

pub(crate) fn create<S, Rt>(
    seq_role: SequencerRole,
    latest_info: StateUpdateInfo<S::Storage>,
    tx_queue_id: Arc<AtomicU64>,
    batch_execution_time_limit_micros: u64,
    seq_config: SequencerConfig<S::Address, PreferredSequencerConfig<S::Address>>,
    shutdown_receiver: watch::Receiver<()>,
    shutdown_sender: watch::Sender<()>,
    executor_events_sender: ExecutorEventsSender<S, Rt>,
    sequence_number_of_next_blob: SequenceNumber,
    in_flight_blobs: Arc<AtomicUsize>,
    stop_at_rollup_height: Option<RollupHeight>,
    rollup_exec_config: RollupBlockExecutorConfig<S>,
    tx_cache_writer: TxResultWriter<S, Rt>,
    cache_warm_up_executor: CacheWarmUpExecutor<S>,
    start_replica_task_notifier: EventReceiverStartNotifier,
) -> (
    SynchronizedSequencerState<S, Rt>,
    SequencerStateUpdator<S, Rt>,
)
where
    S: Spec,
    Rt: Runtime<S>,
{
    let (message_sender, message_receiver) = mpsc::channel(CHANNEL_SIZE);
    let is_ready = Err(SequencerNotReadyDetails::Startup);

    let rate_limiter = SovRateLimiter::new(
        seq_config.sequencer_kind_config.rate_limiter.clone(),
        seq_config
            .sequencer_kind_config
            .batch_execution_time_limit_millis,
        seq_config.max_batch_size_bytes,
    );

    let executor = RollupBlockExecutor::new(
        &latest_info,
        rollup_exec_config.clone(),
        seq_config.clone(),
        Default::default(),
        None, // We'll populate the pinned cache on the first `update_state` call.
    );
    let executor_rebase_height = executor.checkpoint.rollup_height_to_access();

    let inner = Inner {
        seq_role,
        executor,
        executor_rebase_height,
        latest_info,
        tx_queue_id,
        batch_execution_time_limit_micros,
        batch_size_tracker: BatchSizeTracker::new(seq_config.max_batch_size_bytes),
        seq_config: seq_config.clone(),
        shutdown_receiver: shutdown_receiver.clone(),
        shutdown_sender,
        executor_events_sender,
        sequence_number_of_open_batch: None,
        next_unassigned_sequence_number: sequence_number_of_next_blob,
        in_flight_blobs,
        has_finished_startup: false,
        metrics: Vec::with_capacity(128),
        is_ready,
        stop_at_rollup_height,
        rollup_exec_config,
        tx_cache_writer,
        cache_warm_up_executor,
        start_replica_task_notifier,
        rate_limiter,
    };

    let channel_size = Arc::new(AtomicU32::new(0));
    let state = SynchronizedSequencerState {
        inner,
        channel_size: channel_size.clone(),
        message_receiver,
        heap: BTreeMap::new(),
        runtime: Default::default(),
        test_only_state_update_notification_sender: broadcast::channel(100).0,
    };
    let updator = SequencerStateUpdator {
        message_sender,
        channel_size,
        shutdown_receiver,
    };
    (state, updator)
}

type AcceptTxRet<S, Rt> =
    Result<oneshot::Receiver<AcceptedTx<Confirmation<S, Rt>>>, AcceptTxError<S>>;

#[derive(Debug)]
pub(crate) enum AcceptTxError<S: Spec> {
    SequencerOverloaded503,
    NotFullySynced(SequencerNotReadyDetails),
    BatchError {
        batch_creation_error: BatchCreationError,
        nb_of_concurrent_blob_submissions: usize,
    },
    NewTxError(DoNewTxError<S>),
    ReplicaMode,
    RateLimiter(ResourceLimitExceededError<S>),
}

#[derive(Debug)]
pub(crate) struct ProcessFinalCatchupData {
    pub(crate) batches_count: u64,
    pub(crate) transactions_count: usize,
    pub(crate) batch_is_in_progress: bool,
    pub(crate) sequence_number_of_open_batch: Option<SequenceNumber>,
    pub(crate) unprocessed_proofs: BTreeMap<SequenceNumber, PreferredProofToReplay>,
}

#[derive(Debug)]
struct ConditionsTable {
    nodes_sequence_number_is_fresher: bool,
    too_close_to_deferred_slots_count_for_comfort: bool,
    node_is_lagging: bool,
    are_there_batches_to_replay: bool,
    node_is_unsynced_and_doesnt_know_it: bool,
}

#[derive(Debug)]
struct InitialStatus {
    is_startup: bool,
    is_resync: bool,
    is_recover: bool,
}

impl InitialStatus {
    /// After startup, resync, or recovery, the sequencer's in-memory state is no longer guaranteed to be correct and up to date.
    /// When this happens, we replay all soft-confirmed transactions to repopulate the tx and pinned-state caches.
    /// Note: We may be able to optimize away reloading the pinned cache on resync; on startup we have to populate the pinned cache because
    /// it doesn't exist yet, and on recovery we have to relaod it because it was (likely) incorrect - by on simple resync this shouldn't be necessary.
    fn should_flush_tx_cache_and_pinned_cache(&self) -> bool {
        self.is_startup || self.is_resync || self.is_recover
    }
}

/// These two constants are used to calculate the comfortable gas limit.
/// Currently, this is 95% of the initial gas limit. After the comfortable limit is reached,
/// the sequencer will close and publish the current batch.
const COMFORTABLE_GAS_LIMIT_MULTIPLIER: u64 = 19;
const COMFORTABLE_GAS_LIMIT_DIVISOR: u64 = 20;

pub(crate) fn comfortable_gas_limit_for_height<S: Spec>(
    height: RollupHeight,
) -> <S as GasSpec>::Gas {
    let gas_limit = <S as GasSpec>::gas_limit_for_height(height);
    gas_limit
            .scalar_division(COMFORTABLE_GAS_LIMIT_DIVISOR)
            .checked_scalar_product(COMFORTABLE_GAS_LIMIT_MULTIPLIER).unwrap_or_else(|| {
                panic!(
                    "Cannot overflow after dividing by {COMFORTABLE_GAS_LIMIT_DIVISOR} and multiplying by {COMFORTABLE_GAS_LIMIT_MULTIPLIER}",
                )
            })
}
