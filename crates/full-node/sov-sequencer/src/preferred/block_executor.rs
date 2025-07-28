use std::collections::{BTreeMap, VecDeque};
use std::marker::PhantomData;
use std::sync::Arc;

use anyhow::Context;
use axum::http::StatusCode;
use sov_modules_api::capabilities::{
    BlobSelector, BlobSelectorOutput, ChainState, FatalError, RollupHeight,
    TransactionAuthenticator,
};
use sov_modules_api::macros::config_value;
use sov_modules_api::{
    call_message_repr, BlobDataWithId, ChangeSet, DaSpec, ExecutionContext, FullyBakedTx, Gas,
    GasSpec, HexString, KernelStateAccessor, NoOpControlFlow, RejectReason, Runtime,
    RuntimeEventProcessor, RuntimeEventResponse, SelectedBlob, Spec, StateCheckpoint,
    StateUpdateInfo, TransactionReceipt, TxChangeSet, TxHash, VersionReader, VisibleSlotNumber,
};
use sov_modules_stf_blueprint::{BatchReceipt, StfBlueprint};
use sov_rest_utils::{json_obj, ErrorObject};
use sov_state::{Namespace, StateAccesses, StateRoot, Storage};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{self, Sender};
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tracing::trace;
use uuid::Uuid;

use super::state_root_compute::StateRootComputeRequest;
use super::{
    Confirmation, PreferredBatchToReplay, PreferredSequencerConfig, VisibleSlotNumberIncrease,
};
use crate::common::{generic_accept_tx_error, AcceptedTx};
use crate::preferred::async_batch::{ExecutedTxResponse, MaybeAsyncBatch};
use crate::preferred::exit_rollup;
use crate::preferred::transaction_subscriptions::TxResultWriter;
use crate::SequencerConfig;

pub(crate) struct AcceptedTxWithBudgetInfo<S, Rt>
where
    S: Spec,
    Rt: Runtime<S> + RuntimeEventProcessor,
{
    pub accepted_tx: AcceptedTx<Confirmation<S, Rt>>,
    pub remaining_slot_gas: <S as Spec>::Gas,
    pub execution_time_micros: u64,
}

type BlockExecutionOutput<S> = (Vec<BatchReceipt<S>>, ChangeSet, StateAccesses);

#[derive(thiserror::Error, Debug)]
pub(crate) enum RollupBlockExecutorError<S: Spec> {
    #[error(transparent)]
    DecodeCall(#[from] FatalError),
    #[error("The sequencer is temporarily overloaded. Try again in a few seconds")]
    Overloaded,
    #[error("The transaction was rejected: {reason:?} call {call}")]
    Rejected { reason: RejectReason, call: String },
    #[error("The transaction execution was unsuccessful {receipt:?}")]
    UnsuccessfulTransaction { receipt: TransactionReceipt<S> },
    #[error("The rollup block executor task failed unexpectedly")]
    UnexpectedFailure,
}

impl<S: Spec> RollupBlockExecutorError<S> {
    pub fn into_http_error(self) -> ErrorObject {
        match self {
            RollupBlockExecutorError::DecodeCall(_) => ErrorObject {
                status: StatusCode::BAD_REQUEST,
                message: "Malformed transaction".to_string(),
                details: json_obj!({
                    "error": self.to_string(),
                }),
            },
            RollupBlockExecutorError::Overloaded => ErrorObject {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: "Temporarily unavailable".to_string(),
                details: json_obj!({
                    "error": self.to_string(),
                }),
            },
            RollupBlockExecutorError::Rejected { reason, call } => {
                reject_reason_to_error(reason, call)
            }
            RollupBlockExecutorError::UnsuccessfulTransaction { receipt } => {
                generic_accept_tx_error(receipt)
            }
            RollupBlockExecutorError::UnexpectedFailure => ErrorObject {
                status: StatusCode::INTERNAL_SERVER_ERROR,
                message: "Internal Server Error".to_string(),
                details: json_obj!({
                    "error": "The sequencer is shutting down due to an internal error. Your transaction was not accepted.",
                }),
            },
        }
    }
}

type StateRootReceiver<S> =
    oneshot::Receiver<(RollupHeight, <<S as Spec>::Storage as Storage>::Root)>;

pub struct RollupBlockExecutorConfig<S: Spec, Rt: Runtime<S>> {
    pub config: SequencerConfig<S::Address, PreferredSequencerConfig>,
    pub da_address: <S::Da as DaSpec>::Address,
    pub shutdown_notifier: Sender<()>,
    pub state_root_request_sender: tokio::sync::mpsc::Sender<StateRootComputeRequest<S>>,
    pub shutdown_receiver: watch::Receiver<()>,
    pub shutdown_sender: watch::Sender<()>,
    pub startup_transaction_cache_writer: Option<TxResultWriter<S, Rt>>,
}

pub struct RollupBlockExecutor<S, Rt>
where
    S: Spec,
    Rt: Runtime<S>,
{
    pub checkpoint: StateCheckpoint<S>,
    rollup_block_task_state: Option<BackgroundTaskState<S>>,

    next_event_number: u64,
    next_tx_number: u64,
    config: SequencerConfig<S::Address, PreferredSequencerConfig>,
    da_address: <S::Da as DaSpec>::Address,
    // A sender notifying that this acceptor has successfully shut down. We give a handle to
    // each background task when it is spawned, ensuring that this channel remains open as long
    // as any background task is operational even if the acceptor is dropped.
    shutdown_notifier: Sender<()>,
    state_root_request_sender: tokio::sync::mpsc::Sender<StateRootComputeRequest<S>>,
    state_roots: BTreeMap<RollupHeight, <S::Storage as Storage>::Root>,
    state_root_responses: VecDeque<StateRootReceiver<S>>,
    shutdown_receiver: watch::Receiver<()>,
    shutdown_sender: watch::Sender<()>,
    id: Uuid,
    /// A handle to the tx cache for repopulating it on startup. This is `None` except during startup.
    // TODO: This should become part of the "local side effects" task of the executor; passing it here is messy
    startup_transaction_cache_writer: Option<TxResultWriter<S, Rt>>,
    phantom: PhantomData<Rt>,
}

impl<S: Spec, Rt: Runtime<S>> RollupBlockExecutor<S, Rt> {
    /// The maximum number of transactions that can be buffered before incoming txs start getting
    /// rejected.
    const MAX_BUFFERED_TXS: usize = 1;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        info: &StateUpdateInfo<S::Storage>,
        rollup_exec_config: RollupBlockExecutorConfig<S, Rt>,
    ) -> RollupBlockExecutor<S, Rt> {
        let mut rt = Rt::default();
        let checkpoint = StateCheckpoint::new(info.storage.clone(), &rt.kernel());

        let RollupBlockExecutorConfig {
            config,
            da_address,
            shutdown_notifier,
            state_root_request_sender,
            shutdown_receiver,
            shutdown_sender,
            startup_transaction_cache_writer,
        } = rollup_exec_config;

        Self {
            checkpoint,
            rollup_block_task_state: None,
            next_event_number: info.next_event_number,
            next_tx_number: info.next_tx_number,
            config,
            da_address,
            shutdown_notifier,
            state_root_request_sender,
            state_roots: Default::default(),
            state_root_responses: Default::default(),
            id: Uuid::now_v7(),
            shutdown_receiver,
            shutdown_sender,
            startup_transaction_cache_writer,
            phantom: PhantomData,
        }
    }

    pub fn has_in_progress_batch(&self) -> bool {
        self.rollup_block_task_state.is_some()
    }

    #[tracing::instrument(skip_all, level = "trace")]
    pub async fn replace_state(&mut self, other: Self) {
        if self.shutdown_receiver.has_changed().unwrap_or(true) {
            tracing::info!("The sequencer is shutting down. Exiting replace_state");
            return;
        }

        tracing::debug!(old = %self.id, new = %other.id, "Replacing state for block executor");

        if let Some(task_state) = self.rollup_block_task_state.take() {
            task_state.shutdown().abort();
        }

        self.checkpoint = other.checkpoint;
        self.rollup_block_task_state = other.rollup_block_task_state;
        self.next_event_number = other.next_event_number;
        self.next_tx_number = other.next_tx_number;

        // Update our list of state roots from the other executor.
        self.state_roots = other.state_roots;
        self.state_root_responses = other.state_root_responses;
    }

    /// Calls to this method must happen "between"
    /// [`Self::start_rollup_block`] and
    /// [`Self::end_rollup_block`].
    #[tracing::instrument(skip_all, level = "trace")]
    pub async fn apply_tx_to_in_progress_batch(
        &mut self,
        baked_tx: &FullyBakedTx,
    ) -> Result<(AcceptedTxWithBudgetInfo<S, Rt>, TxChangeSet), RollupBlockExecutorError<S>> {
        let result = self.apply_tx_to_in_progress_batch_inner(baked_tx).await;

        match result {
            Ok((receipt, remaining_slot_gas, execution_time_micros, tx_changes)) => {
                let accepted_tx = self.process_tx_receipt(&receipt);
                if let Some(writer) = self.startup_transaction_cache_writer.as_mut() {
                    writer.insert(accepted_tx.clone()).await;
                }
                Ok((
                    AcceptedTxWithBudgetInfo {
                        accepted_tx,
                        remaining_slot_gas,
                        execution_time_micros,
                    },
                    tx_changes,
                ))
            }
            Err(e) => Err(e),
        }
    }

    async fn apply_tx_to_in_progress_batch_inner(
        &mut self,
        baked_tx: &FullyBakedTx,
    ) -> Result<
        (TransactionReceipt<S>, <S as Spec>::Gas, u64, TxChangeSet),
        RollupBlockExecutorError<S>,
    > {
        let Some(task_state) = self.rollup_block_task_state.as_mut() else {
            panic!("Accepting a transaction, yet there's no in-progress batch. This is a bug in the sequencer, please report it.");
        };

        let call = Rt::Auth::decode_serialized_tx(baked_tx)?;
        let call = Rt::wrap_call(call);

        if let Err(TrySendError::Full(_)) = task_state.tx_sender.try_send(baked_tx.clone()) {
            return Err(RollupBlockExecutorError::Overloaded);
        }

        let Some(result) = task_state.result_receiver.recv().await else {
            tracing::error!("The rollup block executor task failed unexpectedly. Gracefully shutting down the sequencer.");
            let _ = self.shutdown_sender.send(()); // We don't care if this fails, because that would mean the sequencer is already shutting down - which is exactly what we want.
            return Err(RollupBlockExecutorError::UnexpectedFailure);
        };

        let ExecutedTxResponse {
            receipt,
            tx_changes,
            remaining_slot_gas,
            execution_time_micros,
        } = result.map_err(|reason| RollupBlockExecutorError::Rejected {
            reason,
            call: call_message_repr::<Rt>(&call),
        })?;

        if !receipt.receipt.is_successful() {
            return Err(RollupBlockExecutorError::UnsuccessfulTransaction { receipt });
        }

        self.checkpoint.apply_changes(tx_changes.0.clone());

        Ok((
            receipt,
            remaining_slot_gas,
            execution_time_micros,
            tx_changes,
        ))
    }

    /// Returns true if [`super::db::PreferredSequencerDb::pop_tx`] ought to be called.
    #[tracing::instrument(skip_all, level = "trace")]
    pub async fn replay_batch(
        &mut self,
        batch: &PreferredBatchToReplay,
        node_state_root: &<S::Storage as Storage>::Root,
    ) -> anyhow::Result<()> {
        self.start_rollup_block_for_replay(
            batch.visible_slot_number_after_increase,
            batch.batch.inner.visible_slots_to_advance,
            node_state_root,
            batch.batch.inner.data.len(),
        )
        .await;

        if self.shutdown_receiver.has_changed().unwrap_or(true) {
            tracing::info!("The sequencer is shutting down. Exiting replay_batch");
            return Ok(());
        }

        for (tx, tx_hash) in batch
            .batch
            .inner
            .data
            .iter()
            .zip(batch.batch.tx_hashes.iter())
        {
            self.replay_tx(*tx_hash, tx).await;
        }

        trace!("Done replaying txs");

        if !batch.is_in_progress {
            self.end_rollup_block().await;
        } else {
            trace!("The batch is still in progress; will keep the background task running");
        }

        Ok(())
    }

    /// A wrapper function for starting batches for replay which makes a few additional safety checks
    /// and does some logging that wouldn't be carried out in the normal case.
    pub(crate) async fn start_rollup_block_for_replay(
        &mut self,
        sanity_check_visible_slot_number_after_increase: VisibleSlotNumber,
        visible_increase: VisibleSlotNumberIncrease,
        // We pass the node state root explicitly because retrieving it is
        // fallible, so it's convenient to front-load the error-checking.
        node_state_root: &<S::Storage as Storage>::Root,
        num_txs: usize,
    ) {
        assert!(
            self.rollup_block_task_state.is_none(),
            "Replaying a preferred batch, but the state is invalid and doesn't allow it ({:?}). This is a bug, please report it.",
            self.rollup_block_task_state
        );

        trace!(num_txs, visible_slot_number_after_increase = %sanity_check_visible_slot_number_after_increase, %node_state_root, "Re-applying batch state changes");

        self.start_rollup_block(
            sanity_check_visible_slot_number_after_increase,
            visible_increase,
            node_state_root,
            // When replaying batches, we wish to be deterministic and not
            // filter out previously-accepted transactions simply because
            // they're not considered profitable enough based on the current
            // configuration value.
            //
            // TODO(@neysofu): write a test for this.
            // TODO(@neysofu): for the very last in-progress batch, this will
            // cause the rest of the batch to not have a minimum profit. We
            // might want to forcibly close that batch and start a new one, or
            // send the new configuration value over a channel.
            0,
        )
        .await;

        trace!("Replaying txs");
    }

    /// A helper function for replaying a transaction that has already been soft-confirmed, logging any errors and exiting.
    /// This differs from `apply_tx_to_in_progress_batch` in that it doesn't return a receipt and
    /// shuts down the rollup on error.
    pub(crate) async fn replay_tx(&mut self, tx_hash: TxHash, tx: &FullyBakedTx) -> u64 {
        trace!(
            %tx_hash,
            "Re-applying state changes for the soft-confirmed transaction"
        );

        match self.apply_tx_to_in_progress_batch(tx).await {
            Ok((output, _tx_changes)) => {
                if tx_hash != output.accepted_tx.tx_hash {
                    tracing::error!(
                        expected_hash = %tx_hash,
                        executor_output_hash = %output.accepted_tx.tx_hash,
                        "The executor returned a different tx hash than expected"
                    );
                    exit_rollup(&self.shutdown_sender).await;
                    unreachable!()
                }
                output.execution_time_micros
            }
            Err(err) => {
                tracing::error!(
                    error = %err,
                    %tx_hash,
                    "Transaction was soft-confirmed but failed to be re-applied; this is a bug, please report it",
                );

                exit_rollup(&self.shutdown_sender).await;
                unreachable!()
            }
        }
    }

    #[tracing::instrument(skip_all, level = "trace")]
    pub async fn start_rollup_block(
        &mut self,
        sanity_check_visible_slot_number_after_increase: VisibleSlotNumber,
        visible_increase: VisibleSlotNumberIncrease,
        // We pass the node state root explicitly because retrieving it is
        // fallible, so it's convenient to front-load the error-checking.
        node_state_root: &<S::Storage as Storage>::Root,
        minimum_profit_per_tx: u128,
    ) {
        assert!(
            self.rollup_block_task_state.is_none(),
            "Starting a rollup block, but there's already one in progress {:?}. This is a bug, please report it.",
            self.rollup_block_task_state
        );

        trace!(
            ?self.checkpoint,
            %visible_increase,
            "Beginning new rollup block and spawning background loop"
        );

        self.populate_state_roots(node_state_root).await;

        let old_visible_slot_number = self.checkpoint.current_visible_slot_number();
        let next_visible_slot_number = self
            .checkpoint
            .current_visible_slot_number()
            .advance(visible_increase.get().into());

        assert_eq!(
            next_visible_slot_number,
            sanity_check_visible_slot_number_after_increase,
            "Sanity check failed: visible slot number calculation was incorrect. This is a bug, please report it."
        );

        let (setup_sender, setup_receiver) = oneshot::channel();
        let (tx_sender, tx_receiver) = mpsc::channel(Self::MAX_BUFFERED_TXS);
        let (result_sender, result_receiver) = mpsc::channel(Self::MAX_BUFFERED_TXS);

        let handle = tokio::runtime::Handle::current().spawn_blocking({
            let ctx = RollupBlockTaskContext {
                checkpoint: self
                    .checkpoint
                    .clone_with_empty_witness_dropping_temp_cache(),
                tx_receiver,
                setup_sender,
                old_visible_slot_number,
                next_visible_slot_number,
                visible_increase,
                result_sender,
                shutdown_notifier: self.shutdown_notifier.clone(),
                old_rollup_height: self.checkpoint.rollup_height_to_access(),
                minimum_profit_per_tx,
                admin_addresses: self.config.admin_addresses.clone().into(),
                sequencer_rollup_address: self.config.rollup_address.clone(),
                sequencer_da_address: self.da_address.clone(),
            };

            move || rollup_block_task_body::<S, Rt>(ctx)
        });

        // Wait for the background task to get up and running, and send the
        // initial change set.
        trace!("Applying setup changes...");
        let setup_changes = setup_receiver
            .await
            .context("Setup must finish successfully")
            .expect("The sequencer can't recover from this error; this is a bug, please report it");
        trace!("Applied setup changes");

        self.checkpoint.apply_changes(setup_changes);
        self.checkpoint
            .advance_visible_slot_number(visible_increase);

        self.rollup_block_task_state = Some(BackgroundTaskState {
            handle,
            tx_sender,
            result_receiver,
        });
    }

    fn process_tx_receipt(
        &mut self,
        tx_receipt: &TransactionReceipt<S>,
    ) -> AcceptedTx<Confirmation<S, Rt>> {
        let tx_number = self.next_tx_number;
        let events = tx_receipt
            .events
            .iter()
            .zip(self.next_event_number..)
            .map(|(event, number)| {
                <RuntimeEventResponse<<Rt as RuntimeEventProcessor>::RuntimeEvent>>::try_from((
                    number, event,
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()
            .expect("Supposedly infallible conversion failed; this is a bug, please report it");

        self.next_event_number += events.len() as u64;
        self.next_tx_number += 1;
        AcceptedTx {
            tx: FullyBakedTx {
                data: tx_receipt.body_to_save.clone().expect(
                    "Transaction receipts contain bodies when using sov-modules-stf-blueprint",
                ),
            },
            tx_hash: tx_receipt.tx_hash,
            confirmation: Confirmation {
                events,
                receipt: tx_receipt.receipt.clone().into(),
                tx_number,
            },
        }
    }

    /// Before starting a rollup block, we need to have stored any visible state roots that it might need in state.
    /// In the node, this is done automatically, but sometimes the sequencer can run too far ahead of the node and need to compute these roots itself.
    async fn populate_state_roots(&mut self, node_state_root: &<S::Storage as Storage>::Root) {
        // If we don't have any state roots yet, insert the node's state root. That's our starting point.
        if self.state_roots.is_empty() {
            self.state_roots.insert(
                self.checkpoint.rollup_height_to_access(),
                node_state_root.clone(),
            );
        }

        // Compute the next visible root height that we need to fetch.
        let next_rollup_height = self.checkpoint.rollup_height_to_access().saturating_add(1);
        let next_visible_rollup_height =
            next_rollup_height.saturating_sub(config_value!("STATE_ROOT_DELAY_BLOCKS"));

        // If we don't have the next visible root locally, fetch it from the background task.
        // Note: The request will *always* be the next one in our inbound queue if we take this branch.
        // If this block is the first one computed using the RollupBlockExecutor, then the root we need is just the one from the node,
        // so we *don't* take this branch.
        // Otherwise, the request will already have been sent during the previous iteration of `end_rollup_block`, so we can just await it here.
        let is_necessary_to_fetch = next_visible_rollup_height
            > *self
                .state_roots
                .keys()
                .max()
                .unwrap_or(&RollupHeight::GENESIS);
        tracing::trace!(
            height= %next_visible_rollup_height,
            "Fetching state root for height, if necessary",
        );
        if is_necessary_to_fetch {
            tracing::trace!(
                fetching_height = %next_visible_rollup_height,
                ?is_necessary_to_fetch,
                "Fetching state root for height",
            );
            let (received_height, next_visible_root) = match self.state_root_responses.pop_front().unwrap_or_else(||
                    panic!("Executor {} needed response for state root for height {} before sending request. This is a bug in the `RollupBlockExecutor`, please report it.", self.id, next_visible_rollup_height))
            .await {
                Ok((received_height, next_visible_root)) => {
                   (received_height, next_visible_root)
                }
                Err(_) => {
                    tracing::info!("State root computation background task has shutdown. New block not started");
                    return;
                }
            };
            // Sanity check: the height we received should match the height we need.
            if received_height != next_visible_rollup_height {
                tracing::error!(
                    received_height = %received_height,
                    next_visible_root_height = %next_visible_rollup_height,
                    "Received height did not equal expected height for assertion. This is a bug in the RollupBlockExecutor, please report it.");
                panic!("Received height ({received_height}) did not equal expected height for assertion {next_visible_rollup_height}. This is a bug in the RollupBlockExecutor, please report it.");
            }
            tracing::trace!(
                "Received state root for height {} : {}",
                next_visible_rollup_height,
                HexString(next_visible_root.namespace_root(sov_state::ProvableNamespace::User))
            );
            self.state_roots
                .insert(next_visible_rollup_height, next_visible_root);
        }
        // take all roots greater than self.started_from
        for (height, root) in self.state_roots.iter() {
            let user_root = root.namespace_root(sov_state::ProvableNamespace::User);
            let mut runtime = Rt::default();
            let mut kernel = runtime.kernel();
            let mut kernel_state =
                KernelStateAccessor::from_checkpoint(&kernel, &mut self.checkpoint);
            kernel.save_user_state_root(*height, user_root, &mut kernel_state);
        }
    }

    #[tracing::instrument(skip_all, level = "trace")]
    pub async fn end_rollup_block(&mut self) {
        trace!("Ending rollup block");

        let task_state = self
            .rollup_block_task_state
            .take()
            .expect("No in-progress rollup block, nothing to do. This is a bug, please report it");

        let rollup_height = self.checkpoint.rollup_height_to_access();
        let (batch_receipts, changes, state_accesses) = task_state.shutdown().await.expect(
            "Transaction acceptor task failed unexpectedly! This is a bug, please report it.",
        );

        let mut accepted_txs_by_batch = Vec::with_capacity(batch_receipts.len());
        for batch_receipt in batch_receipts {
            // We already increment the event number for our own transactions
            // inside `apply_tx_to_in_progress_batch`.
            if batch_receipt.inner.da_address == self.da_address {
                continue;
            }
            let mut accepted_txs = Vec::with_capacity(batch_receipt.tx_receipts.len());
            for tx_receipt in batch_receipt.tx_receipts {
                let accepted_tx = self.process_tx_receipt(&tx_receipt);
                accepted_txs.push(accepted_tx);
            }
            accepted_txs_by_batch.push(accepted_txs);
        }

        trace!(
            executor_id = %self.id,
            %rollup_height,
            user_writes = %state_accesses.user.ordered_writes.len(),
            kernel_writes = %state_accesses.kernel.ordered_writes.len(),
            "Sending state root computation request to background task");
        let (response_channel, response_receiver) = oneshot::channel();
        self.state_root_responses.push_back(response_receiver);

        if self
            .state_root_request_sender
            .send(StateRootComputeRequest {
                state_accesses,
                storage: self.checkpoint.storage().clone(),
                rollup_height,
                max_slot_number: self.checkpoint.max_allowed_slot_number_to_access(),
                response_channel,
            })
            .await
            .is_err()
        {
            tracing::info!(executor_id = %self.id, "State root computation background task has shutdown. State root will not be computed.");
        }

        self.checkpoint.apply_changes(changes);

        trace!(%rollup_height, "Successfully ended rollup block");
    }
}

#[derive(Debug)]
struct BackgroundTaskState<S: Spec> {
    handle: JoinHandle<BlockExecutionOutput<S>>,
    tx_sender: mpsc::Sender<FullyBakedTx>,
    result_receiver: mpsc::Receiver<Result<ExecutedTxResponse<S>, RejectReason>>,
}

impl<S: Spec> BackgroundTaskState<S> {
    fn shutdown(self) -> JoinHandle<BlockExecutionOutput<S>> {
        // Must be dropped before the result receiver, or a deadlock happens.
        drop(self.tx_sender);
        self.handle
    }
}

struct RollupBlockTaskContext<S: Spec> {
    checkpoint: StateCheckpoint<S>,
    old_rollup_height: RollupHeight,
    old_visible_slot_number: VisibleSlotNumber,
    next_visible_slot_number: VisibleSlotNumber,
    visible_increase: VisibleSlotNumberIncrease,
    // Channels
    // --------
    tx_receiver: mpsc::Receiver<FullyBakedTx>,
    setup_sender: oneshot::Sender<ChangeSet>,
    result_sender: mpsc::Sender<Result<ExecutedTxResponse<S>, RejectReason>>,
    shutdown_notifier: mpsc::Sender<()>,
    // Config values
    // --------
    minimum_profit_per_tx: u128,
    admin_addresses: Arc<Vec<S::Address>>,
    sequencer_rollup_address: S::Address,
    sequencer_da_address: <S::Da as DaSpec>::Address,
}

fn rollup_block_task_body<S, Rt>(ctx: RollupBlockTaskContext<S>) -> BlockExecutionOutput<S>
where
    S: Spec,
    Rt: Runtime<S>,
{
    let RollupBlockTaskContext {
        mut checkpoint,
        old_rollup_height,
        old_visible_slot_number,
        next_visible_slot_number,
        visible_increase,
        tx_receiver,
        setup_sender,
        result_sender,
        shutdown_notifier,
        minimum_profit_per_tx,
        admin_addresses,
        sequencer_rollup_address,
        sequencer_da_address,
    } = ctx;

    let _span = tracing::trace_span!(
        "preferred_seq_bg_task",
        checkpoint_height = %checkpoint.rollup_height_to_access(),
    )
    .entered();

    let stf = StfBlueprint::<S, Rt>::new();
    let mut rt = Rt::default();
    let mut kernel = rt.kernel();
    let mut accessor: KernelStateAccessor<'_, S> =
        KernelStateAccessor::from_checkpoint(&kernel, &mut checkpoint);

    kernel.increment_rollup_height(&mut accessor, next_visible_slot_number);

    let target_rollup_height = old_rollup_height.saturating_add(1);
    let next_root = kernel
        .visible_hash_for(target_rollup_height, &mut accessor)
        .ok_or_else(|| format!("Can't get visible hash for {target_rollup_height}"))
        .unwrap();
    // Now that we've incremented the rollup height, we can get the next gas price. Do that and use it to compute the amount of funds that we should
    // reserve for the preferred sequencer.
    let next_gas_price = kernel
        .base_fee_per_gas(&mut accessor)
        .unwrap_or(S::initial_base_fee_per_gas());
    let needed_gas_escrow = S::max_tx_check_costs()
        .checked_value(&next_gas_price)
        .expect("Gas price overflow! This is a bug, please report it.");
    kernel.escrow_funds_for_preferred_sequencer(needed_gas_escrow, &mut accessor).expect("Failed to escrow funds for the preferred sequencer. The sequencer is too low on funds, which could cause soft confirmations to be invalidated. Increase your bond and restart the sequencer.");

    let blob_selector_output = {
        let preferred_blob = SelectedBlob {
            blob_data: BlobDataWithId::Batch(MaybeAsyncBatch::new_async(
                tx_receiver,
                setup_sender,
                result_sender,
                minimum_profit_per_tx,
                admin_addresses,
                sequencer_rollup_address,
            )),
            reserved_gas_tokens: Some(needed_gas_escrow),
            sender: sequencer_da_address.clone(),
        };

        let non_preferred_blobs = kernel
            .get_non_preferred_blobs(
                old_visible_slot_number
                    .as_true()
                    .next()
                    .range_inclusive(next_visible_slot_number.as_true()),
                &mut accessor,
                NoOpControlFlow,
            )
            .into_iter()
            .map(|mut b| {
                // Batches from unregistered sequencers don't reserve any gas
                // tokens.
                if b.reserved_gas_tokens.is_some() {
                    b.reserved_gas_tokens = Some(needed_gas_escrow);
                }
                b.map_batch(MaybeAsyncBatch::<S>::new_sync)
            })
            .collect::<Vec<_>>();

        tracing::debug!(count = %non_preferred_blobs.len(), "Extracted non-preferred blobs");

        let mut selected_blobs = vec![preferred_blob];
        selected_blobs.extend(non_preferred_blobs);

        BlobSelectorOutput {
            selected_blobs,
            visible_slot_number_increase: visible_increase.get().into(),
        }
    };

    tracing::trace!(
        %next_visible_slot_number,
        "Applying batches in user space"
    );
    let (_, _, batch_receipts, mut checkpoint) = stf.apply_batches_in_user_space(
        &mut Default::default(),
        blob_selector_output,
        checkpoint,
        ExecutionContext::Sequencer,
        next_root,
    );

    let mut changes = checkpoint.changes();
    let (accessory_delta, state_accesses, _witness) =
        stf.materialize_accessory_state(&mut Default::default(), checkpoint);

    changes.changes.extend(
        accessory_delta
            .freeze()
            .into_iter()
            .map(|(k, v)| ((k.clone(), Namespace::Accessory), v.clone())),
    );
    drop(shutdown_notifier);

    (batch_receipts, changes, state_accesses)
}

fn reject_reason_to_error(
    error: RejectReason,
    call_discriminant: impl std::fmt::Debug,
) -> ErrorObject {
    match error {
        RejectReason::SequencerOutOfGas => ErrorObject {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: "Batch is full".to_string(),
            details: json_obj!({
                "error": "More transactions were submitted that the sequencer is allowed to put into a single batch. Wait a few seconds and try again."
            }),
        },
        RejectReason::InsufficientReward { expected, found } => ErrorObject {
            status: StatusCode::FORBIDDEN,
            message: "Sequencer tip too low".to_string(),
            details: json_obj!({
                "error": "This transaction did not pay a sufficient net fee.",
                "minimum": expected,
                "found": found,
            }),
        },
        RejectReason::SenderMustBeAdmin => ErrorObject {
            status: StatusCode::FORBIDDEN,
            message: "The transaction is forbidden".to_string(),
            details: json_obj!({
                "error": format!("Only designated admins are allowed to send `{:#?}` transactions through this sequencer", call_discriminant),
            }),
        },
    }
}
