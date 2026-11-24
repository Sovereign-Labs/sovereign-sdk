use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use sov_modules_api::state::TxScratchpad;
use sov_modules_api::{
    ChangeSet, Context, DispatchCall, ExecutionContext, FullyBakedTx, Gas, GasArray,
    IncrementalBatch, InjectedControlFlow, IterableBatchWithId, MaybeExecuted, NoOpControlFlow,
    ProvisionalSequencerOutcome, Runtime, SlotGasMeter, TransactionReceipt, TxChangeSet,
    TxControlFlow,
};
use tokio::runtime::Handle;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot;

use super::{gas_reached_comfortable_limit, BatchSizeTracker, RejectReason, Spec, StateCheckpoint};
use crate::common::sender_is_allowed;

/// How much more the open batch may grow before the actor closes it. Computed by the actor
/// at the start of each chunk of pipelined txs, from the same thresholds it checks after every
/// successful tx.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CloseBudget {
    /// Serialized bytes (as counted by [`BatchSizeTracker`]) that fit under the size threshold.
    pub(crate) bytes_room: u64,
    /// Execution time that fits under the execution time threshold.
    pub(crate) execution_time_room_micros: u64,
}

/// Marks where a chunk of pipelined txs begins.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ChunkMarker {
    /// The tx belongs to the chunk started by an earlier tx.
    Continue,
    /// The tx starts a new chunk. Once a successful tx in the chunk crosses a batch-close
    /// threshold, the executor stops and returns the rest of the chunk as
    /// [`ExecutorOutput::Deferred`].
    Start(CloseBudget),
    /// The tx starts a new chunk that is never stopped. Used when txs are sent one at a time.
    Unbudgeted,
}

/// A tx sent to the executor, together with its position in a chunk.
#[derive(Debug)]
pub(crate) struct ExecutorTx {
    pub(crate) tx: FullyBakedTx,
    pub(crate) chunk: ChunkMarker,
}

/// What the executor sends back for each [`ExecutorTx`], in the order the txs were sent.
pub(crate) enum ExecutorOutput<S: Spec> {
    Executed(AsyncBatchResult<S>),
    /// The executor stopped the chunk before this tx, because an earlier tx crossed a
    /// batch-close threshold. The tx was not executed.
    Deferred(FullyBakedTx),
}

/// The executor's view of the current chunk. It mirrors the batch-close checks of
/// `Inner::should_close_batch`, so that the executor never runs a tx that the actor
/// would put into the next batch.
#[derive(Debug, Default)]
struct ChunkState {
    budget: Option<CloseBudget>,
    used_bytes: u64,
    used_execution_time_micros: u64,
    /// The length of the tx being executed, recorded when it is handed to the STF.
    current_tx_len: usize,
    stopped: bool,
}

impl ChunkState {
    fn start(&mut self, budget: Option<CloseBudget>) {
        *self = Self {
            budget,
            ..Default::default()
        };
    }

    /// Accounts for a tx whose changes were committed to the batch.
    ///
    /// The actor counts the tx length after sequencing data is pruned, which is never larger than
    /// the length counted here. So the executor stops at or before the tx after which the actor
    /// closes the batch, never later.
    fn record_committed_tx<S: Spec>(
        &mut self,
        execution_time_micros: u64,
        remaining_slot_gas: <S as Spec>::Gas,
    ) {
        let Some(budget) = self.budget else {
            return;
        };
        self.used_bytes = self
            .used_bytes
            .saturating_add(BatchSizeTracker::serialized_tx_size(self.current_tx_len) as u64);
        self.used_execution_time_micros = self
            .used_execution_time_micros
            .saturating_add(execution_time_micros);
        self.stopped = self.used_bytes > budget.bytes_room
            || self.used_execution_time_micros > budget.execution_time_room_micros
            || gas_reached_comfortable_limit::<S>(remaining_slot_gas);
    }
}

/// A batch that might be received async from some producer
#[derive(Debug)]
pub enum MaybeAsyncBatch<S: Spec> {
    /// The batch is streamed from a channel.
    Async {
        txs_receiver: Receiver<ExecutorTx>,
        responder: AsyncBatchResponder<S>,
        setup_sender: Option<oneshot::Sender<ChangeSet>>,
        address: S::Address,
    },
    /// Batch contents are fully known ahead of time.
    Sync {
        batch: IterableBatchWithId<S, NoOpControlFlow>,
    },
}

impl<S: Spec> MaybeAsyncBatch<S> {
    /// Create a new batch with a receiver for transactions.
    pub(crate) fn new_async(
        txs_receiver: Receiver<ExecutorTx>,
        setup_sender: oneshot::Sender<ChangeSet>,
        result_channel: Sender<ExecutorOutput<S>>,
        tx_profit_threshold: u128,
        sequencer_admins: Arc<Vec<S::Address>>,
        address: S::Address,
        is_responsible_for_gating_admins: bool,
    ) -> Self {
        Self::Async {
            txs_receiver,
            address,
            setup_sender: Some(setup_sender),
            responder: AsyncBatchResponder {
                result_channel,
                admins: sequencer_admins,
                tx_profit_threshold,
                // This will get overwritten by the pre-flight hook.
                execution_start: std::cell::RefCell::new(std::time::Instant::now()),
                is_responsible_for_gating_admins,
                chunk: Rc::default(),
            },
        }
    }

    /// Create a MaybeAsyncBatch from a batch whose contents are already known.
    pub fn new_sync(batch: IterableBatchWithId<S, NoOpControlFlow>) -> Self {
        Self::Sync { batch }
    }
}

/// The control flow injector for an async batch.
#[derive(Debug)]
pub enum MaybeAsyncBatchControlFlow<S: Spec> {
    Async { responder: AsyncBatchResponder<S> },
    Sync,
}

impl<S: Spec> InjectedControlFlow<S> for MaybeAsyncBatchControlFlow<S> {
    fn post_tx(
        &self,
        provisional_outcome: ProvisionalSequencerOutcome<S>,
        dirty_scratchpad: TxScratchpad<S, StateCheckpoint<S>>,
        slot_gas_meter_before_tx: &SlotGasMeter<S>,
        gas_used: <S as Spec>::Gas,
        execution_context: ExecutionContext,
    ) -> (StateCheckpoint<S>, TxControlFlow<TransactionReceipt<S>>) {
        match self {
            Self::Async { responder } => responder.post_tx(
                provisional_outcome,
                dirty_scratchpad,
                slot_gas_meter_before_tx,
                gas_used,
                execution_context,
            ),
            Self::Sync => <NoOpControlFlow as sov_modules_api::InjectedControlFlow<S>>::post_tx(
                &NoOpControlFlow,
                provisional_outcome,
                dirty_scratchpad,
                slot_gas_meter_before_tx,
                gas_used,
                execution_context,
            ),
        }
    }

    fn pre_flight<RT: Runtime<S>>(
        &self,
        runtime: &RT,
        context: &Context<S>,
        call: &<RT as DispatchCall>::Decodable,
    ) -> TxControlFlow<()> {
        match self {
            Self::Async { responder } => responder.pre_flight(runtime, context, call),
            Self::Sync => <NoOpControlFlow as InjectedControlFlow<S>>::pre_flight(
                &NoOpControlFlow,
                runtime,
                context,
                call,
            ),
        }
    }
}

/// The response from the sequencer for a submitted transaction, along with the resources consumed by that transaction.
pub(crate) struct AsyncBatchResult<S: Spec> {
    pub(crate) execution_time_micros: u64,
    pub(crate) gas_used: <S as Spec>::Gas,
    pub(crate) inner_result: Result<ExecutedTxResponse<S>, RejectReason>,
}

/// The response from the sequencer to a submitted tx that was actually executed. Note that this
/// transaction may not have been successful!
pub(crate) struct ExecutedTxResponse<S: Spec> {
    pub(crate) receipt: TransactionReceipt<S>,
    pub(crate) tx_changes: TxChangeSet,
    pub(crate) remaining_slot_gas: <S as Spec>::Gas,
    pub(crate) sequencing_scratchpad: sov_modules_api::SequencingScratchpadContents,
}

/// The channel responsible for notifying an async tx submitter of the txs result
#[derive(Debug)]
pub struct AsyncBatchResponder<S: Spec> {
    result_channel: Sender<ExecutorOutput<S>>,
    admins: Arc<Vec<S::Address>>,
    tx_profit_threshold: u128,
    execution_start: std::cell::RefCell<std::time::Instant>,
    is_responsible_for_gating_admins: bool,
    chunk: Rc<RefCell<ChunkState>>,
}

impl<S: Spec> AsyncBatchResponder<S> {
    fn send_async_batch_result(&self, item: ExecutorOutput<S>) {
        // Try a simple non-blocking send first, then fall back to blocking the runtime if that fails
        if let Err(TrySendError::Full(item)) = self.result_channel.try_send(item) {
            let _ = Handle::current().block_on(async move { self.result_channel.send(item).await });
        }
    }

    fn send(
        &self,
        gas_used: <S as Spec>::Gas,
        execution_time_micros: u64,
        inner_result: Result<ExecutedTxResponse<S>, RejectReason>,
    ) {
        self.send_async_batch_result(ExecutorOutput::Executed(AsyncBatchResult {
            gas_used,
            execution_time_micros,
            inner_result,
        }));
    }

    /// Create a new responder for a single tx that shares the same config as the old value. Note that the timestamp is a fresh atomic initialized to zero
    /// This is fine, since it will be reset again by the pre-flight hook.
    fn clone_for_tx(&self) -> Self {
        Self {
            result_channel: self.result_channel.clone(),
            admins: self.admins.clone(),
            tx_profit_threshold: self.tx_profit_threshold,
            execution_start: std::cell::RefCell::new(std::time::Instant::now()),
            is_responsible_for_gating_admins: self.is_responsible_for_gating_admins,
            chunk: self.chunk.clone(),
        }
    }
}

impl<S: Spec> AsyncBatchResponder<S> {
    fn pre_flight<RT: Runtime<S>>(
        &self,
        runtime: &RT,
        context: &Context<S>,
        call: &<RT as DispatchCall>::Decodable,
    ) -> TxControlFlow<()> {
        self.execution_start.replace(std::time::Instant::now());
        if !self.is_responsible_for_gating_admins
            || sender_is_allowed(
                runtime,
                call,
                context.sender(),
                context.sequencer_da_address(),
                self.admins.as_slice(),
            )
        {
            TxControlFlow::ContinueProcessing(())
        } else {
            let execution_time_micros =
                self.execution_start.borrow().elapsed().as_micros().try_into().expect("Unix time in micros overflowed u64. This should be unreachable for the next 300,000 years");
            // This is the only response for this tx: the STF does not call `post_tx` for txs
            // ignored by the pre-flight hook.
            self.send(
                S::Gas::zero(),
                execution_time_micros,
                Err(RejectReason::SenderMustBeAdmin),
            );
            TxControlFlow::IgnoreTx
        }
    }

    fn post_tx(
        &self,
        provisional_outcome: ProvisionalSequencerOutcome<S>,
        dirty_scratchpad: TxScratchpad<S, StateCheckpoint<S>>,
        slot_gas_meter_before_tx: &SlotGasMeter<S>,
        gas_used: <S as Spec>::Gas,
        _execution_context: ExecutionContext,
    ) -> (StateCheckpoint<S>, TxControlFlow<TransactionReceipt<S>>) {
        let execution_time_micros =
            self.execution_start.borrow().elapsed().as_micros().try_into().expect("Unix time in micros overflowed u64. This should be unreachable for the next 300,000 years");
        let ProvisionalSequencerOutcome {
            reward,
            penalty,
            execution_status,
            sequencing_scratchpad,
        } = provisional_outcome;
        let MaybeExecuted::Executed(receipt) = execution_status else {
            self.send(
                gas_used,
                execution_time_micros,
                Err(RejectReason::SequencerOutOfGas),
            );
            return (dirty_scratchpad.revert(), TxControlFlow::IgnoreTx);
        };

        if !receipt.receipt.is_successful() {
            let response = ExecutedTxResponse {
                receipt: receipt.clone(),
                tx_changes: dirty_scratchpad.tx_changes(),
                remaining_slot_gas: *slot_gas_meter_before_tx.remaining_preferred_slot_gas(), // Since we ignore this tx, the remaining gas limit is unchanged
                sequencing_scratchpad,
            };

            self.send(gas_used, execution_time_micros, Ok(response));
            return (dirty_scratchpad.revert(), TxControlFlow::IgnoreTx);
        }

        if penalty > reward || reward.saturating_sub(penalty) < self.tx_profit_threshold {
            self.send(
                gas_used,
                execution_time_micros,
                Err(RejectReason::InsufficientReward {
                    expected: self.tx_profit_threshold,
                    found: reward.saturating_sub(penalty).0,
                }),
            );

            return (dirty_scratchpad.revert(), TxControlFlow::IgnoreTx);
        }

        let remaining_slot_gas = (*slot_gas_meter_before_tx.remaining_preferred_slot_gas())
            .checked_sub(gas_used)
            // SAFETY: We always enforce that the gas used is less than the remaining slot gas limit
            .expect("Impossible happened: SlotGasMeter underflow when charging gas.");

        let response = ExecutedTxResponse {
            receipt: receipt.clone(),
            tx_changes: dirty_scratchpad.tx_changes(),
            remaining_slot_gas,
            sequencing_scratchpad,
        };

        self.chunk
            .borrow_mut()
            .record_committed_tx::<S>(execution_time_micros, remaining_slot_gas);
        self.send(gas_used, execution_time_micros, Ok(response));
        (
            dirty_scratchpad.commit(),
            TxControlFlow::ContinueProcessing(receipt),
        )
    }
}

impl<S: Spec> IncrementalBatch<S> for MaybeAsyncBatch<S> {
    type ControlFlow = MaybeAsyncBatchControlFlow<S>;

    fn known_remaining_txs(&self) -> Option<usize> {
        match self {
            MaybeAsyncBatch::Async { .. } => None,
            MaybeAsyncBatch::Sync { batch } => Some(batch.remaining()),
        }
    }

    fn id(&self) -> Option<[u8; 32]> {
        match self {
            MaybeAsyncBatch::Async { .. } => None,
            MaybeAsyncBatch::Sync { batch } => Some(batch.id),
        }
    }

    fn pre_flight(&mut self, state_checkpoint: &mut StateCheckpoint<S>) {
        match self {
            MaybeAsyncBatch::Async { setup_sender, .. } => {
                let changes = state_checkpoint.changes();
                // If the receiver is no longer available, we don't care about sending the changes.
                let _  = setup_sender.take().expect("The pre-flight hook of a single batch was invoked multiple times! This is a bug - please report it.").send(changes);
            }
            MaybeAsyncBatch::Sync { .. } => {}
        }
    }

    fn sequencer_address(&self) -> S::Address {
        match self {
            MaybeAsyncBatch::Async { address, .. } => *address,
            MaybeAsyncBatch::Sync { batch } => batch.sequencer_address,
        }
    }
}

/// How long the executor thread polls for the next tx before parking. Under load the next tx
/// arrives within tens of microseconds, and a park/unpark round trip costs a large share of that.
const SPIN_BEFORE_PARK: std::time::Duration = std::time::Duration::from_micros(50);

fn recv_spinning_first(txs_receiver: &mut Receiver<ExecutorTx>) -> Option<ExecutorTx> {
    let spin_until = std::time::Instant::now() + SPIN_BEFORE_PARK;
    loop {
        match txs_receiver.try_recv() {
            Ok(item) => return Some(item),
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => return None,
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                if std::time::Instant::now() >= spin_until {
                    return Handle::current().block_on(txs_receiver.recv());
                }
                std::hint::spin_loop();
            }
        }
    }
}

impl<S: Spec> Iterator for MaybeAsyncBatch<S> {
    type Item = (FullyBakedTx, MaybeAsyncBatchControlFlow<S>);

    fn next(&mut self) -> Option<(FullyBakedTx, MaybeAsyncBatchControlFlow<S>)> {
        // Get a handle to the current runtime, then block on receiving an update
        // from the channel. This is coupled to the implementation of the sequencer,
        // which requires that the apply_slot function be spawned on a blocking thread
        match self {
            MaybeAsyncBatch::Async {
                txs_receiver,
                responder,
                ..
            } => loop {
                let ExecutorTx { tx, chunk } = recv_spinning_first(txs_receiver)?;
                let mut chunk_state = responder.chunk.borrow_mut();
                match chunk {
                    ChunkMarker::Continue => {}
                    ChunkMarker::Start(budget) => chunk_state.start(Some(budget)),
                    ChunkMarker::Unbudgeted => chunk_state.start(None),
                }
                if chunk_state.stopped {
                    drop(chunk_state);
                    responder.send_async_batch_result(ExecutorOutput::Deferred(tx));
                    continue;
                }
                chunk_state.current_tx_len = tx.len();
                drop(chunk_state);
                break Some((
                    tx,
                    MaybeAsyncBatchControlFlow::Async {
                        responder: responder.clone_for_tx(),
                    },
                ));
            },
            MaybeAsyncBatch::Sync { batch } => batch
                .next()
                .map(|(item, _)| (item, MaybeAsyncBatchControlFlow::Sync)),
        }
    }
}

#[cfg(test)]
mod tests {
    use sov_modules_api::GasSpec;
    use sov_test_utils::TestSpec;

    use super::*;

    const TX_LEN: usize = 1_000;

    fn plenty_of_gas() -> <TestSpec as Spec>::Gas {
        <TestSpec as GasSpec>::block_gas_limit()
    }

    fn chunk_with_budget(bytes_room: u64, execution_time_room_micros: u64) -> ChunkState {
        let mut chunk = ChunkState::default();
        chunk.start(Some(CloseBudget {
            bytes_room,
            execution_time_room_micros,
        }));
        chunk.current_tx_len = TX_LEN;
        chunk
    }

    #[test]
    fn stops_after_the_tx_that_exceeds_the_bytes_room() {
        let tx_size = BatchSizeTracker::serialized_tx_size(TX_LEN) as u64;
        let mut chunk = chunk_with_budget(2 * tx_size, u64::MAX);

        chunk.record_committed_tx::<TestSpec>(300, plenty_of_gas());
        chunk.record_committed_tx::<TestSpec>(300, plenty_of_gas());
        assert!(
            !chunk.stopped,
            "Filling the bytes room exactly must not stop the chunk"
        );

        chunk.record_committed_tx::<TestSpec>(300, plenty_of_gas());
        assert!(
            chunk.stopped,
            "Exceeding the bytes room must stop the chunk"
        );
    }

    #[test]
    fn stops_after_the_tx_that_exceeds_the_execution_time_room() {
        let mut chunk = chunk_with_budget(u64::MAX, 5_000);

        chunk.record_committed_tx::<TestSpec>(2_500, plenty_of_gas());
        chunk.record_committed_tx::<TestSpec>(2_500, plenty_of_gas());
        assert!(
            !chunk.stopped,
            "Filling the execution time room exactly must not stop the chunk"
        );

        chunk.record_committed_tx::<TestSpec>(1, plenty_of_gas());
        assert!(
            chunk.stopped,
            "Exceeding the execution time room must stop the chunk"
        );
    }

    #[test]
    fn stops_once_gas_reaches_the_comfortable_limit() {
        let mut chunk = chunk_with_budget(u64::MAX, u64::MAX);

        chunk.record_committed_tx::<TestSpec>(300, <TestSpec as Spec>::Gas::zero());

        assert!(chunk.stopped, "Running out of slot gas must stop the chunk");
    }

    #[test]
    fn unbudgeted_chunk_never_stops() {
        let mut chunk = ChunkState::default();
        chunk.start(None);
        chunk.current_tx_len = TX_LEN;

        chunk.record_committed_tx::<TestSpec>(u64::MAX, <TestSpec as Spec>::Gas::zero());

        assert!(!chunk.stopped);
    }

    #[test]
    fn starting_a_chunk_clears_the_stop() {
        let mut chunk = chunk_with_budget(u64::MAX, u64::MAX);
        chunk.record_committed_tx::<TestSpec>(300, <TestSpec as Spec>::Gas::zero());
        assert!(chunk.stopped);

        chunk.start(Some(CloseBudget {
            bytes_room: u64::MAX,
            execution_time_room_micros: u64::MAX,
        }));

        assert!(!chunk.stopped);
    }
}
