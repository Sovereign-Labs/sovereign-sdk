use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sov_modules_api::state::TxScratchpad;
use sov_modules_api::{
    ChangeSet, Context, DispatchCall, FullyBakedTx, GasArray, IncrementalBatch,
    InjectedControlFlow, IterableBatchWithId, MaybeExecuted, NoOpControlFlow,
    ProvisionalSequencerOutcome, Runtime, SlotGasMeter, TransactionReceipt, TxChangeSet,
    TxControlFlow,
};
use tokio::runtime::Handle;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::oneshot;

use super::{RejectReason, Spec, StateCheckpoint};
use crate::common::sender_is_allowed;

/// A batch that might be received async from some producer
#[derive(Debug)]
pub enum MaybeAsyncBatch<S: Spec> {
    /// The batch is streamed from a channel.
    Async {
        txs_receiver: Receiver<FullyBakedTx>,
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
    pub fn new_async(
        txs_receiver: Receiver<FullyBakedTx>,
        setup_sender: oneshot::Sender<ChangeSet>,
        result_channel: Sender<Result<ExecutedTxResponse<S>, RejectReason>>,
        tx_profit_threshold: u128,
        sequencer_admins: Arc<Vec<S::Address>>,
        address: S::Address,
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
                unix_timestamp_micros: AtomicU64::new(0),
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
    Async(AsyncBatchResponder<S>),
    Sync,
}

impl<S: Spec> InjectedControlFlow<S> for MaybeAsyncBatchControlFlow<S> {
    fn post_tx(
        &self,
        provisional_outcome: ProvisionalSequencerOutcome<S>,
        dirty_scratchpad: TxScratchpad<S, StateCheckpoint<S>>,
        slot_gas_meter_before_tx: &SlotGasMeter<S>,
        gas_used: &<S as Spec>::Gas,
    ) -> (StateCheckpoint<S>, TxControlFlow<TransactionReceipt<S>>) {
        match self {
            Self::Async(responder) => responder.post_tx(
                provisional_outcome,
                dirty_scratchpad,
                slot_gas_meter_before_tx,
                gas_used,
            ),
            Self::Sync => <NoOpControlFlow as sov_modules_api::InjectedControlFlow<S>>::post_tx(
                &NoOpControlFlow,
                provisional_outcome,
                dirty_scratchpad,
                slot_gas_meter_before_tx,
                gas_used,
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
            Self::Async(responder) => responder.pre_flight(runtime, context, call),
            Self::Sync => <NoOpControlFlow as InjectedControlFlow<S>>::pre_flight(
                &NoOpControlFlow,
                runtime,
                context,
                call,
            ),
        }
    }
}

/// The response from the sequencer to a submitted tx that was actually executed. Note that this
/// transaction may not have been successful!
pub(crate) struct ExecutedTxResponse<S: Spec> {
    pub receipt: TransactionReceipt<S>,
    pub tx_changes: TxChangeSet,
    pub remaining_slot_gas: <S as Spec>::Gas,
    pub execution_time_micros: u64,
}

/// The channel responsible for notifying an async tx submitter of the txs result
#[derive(Debug)]
pub struct AsyncBatchResponder<S: Spec> {
    result_channel: Sender<Result<ExecutedTxResponse<S>, RejectReason>>,
    admins: Arc<Vec<S::Address>>,
    tx_profit_threshold: u128,
    /// The timestamp of the start of the latest tx in microseconds since the UNIX epoch
    /// We use an atomic u64 to avoid requiring a mutex. Note that this is set during the pre-flight hook.
    /// and read during the post-tx hook. It may not be meaningful before the pre-flight hook is called.
    unix_timestamp_micros: AtomicU64,
}

impl<S: Spec> AsyncBatchResponder<S> {
    fn send_item(&self, item: Result<ExecutedTxResponse<S>, RejectReason>) {
        // Try a simple non-blocking send first, then fall back to blocking the runtime if that fails
        if let Err(TrySendError::Full(item)) = self.result_channel.try_send(item) {
            let _ = Handle::current().block_on(async move { self.result_channel.send(item).await });
        }
    }

    /// Create a new responder for a single tx that shares the same config as the old value. Note that the timestamp is a fresh atomic initialized to zero
    /// This is fine, since it will be reset again by the pre-flight hook.
    fn clone_for_tx(&self) -> Self {
        Self {
            result_channel: self.result_channel.clone(),
            admins: self.admins.clone(),
            tx_profit_threshold: self.tx_profit_threshold,
            unix_timestamp_micros: AtomicU64::new(0),
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
        let start_time: u64 = SystemTime::now().duration_since(UNIX_EPOCH).expect("SystemTime::now() returned something earlier than the UNIX epoch. This should be unreachable.").as_micros().try_into().expect("Unix time in micros overflowed u64. This should be unreachable for the next 300,000 years");
        self.unix_timestamp_micros
            .store(start_time, Ordering::SeqCst);
        if sender_is_allowed(
            runtime,
            call,
            context.sender(),
            context.sequencer_da_address(),
            self.admins.as_slice(),
        ) {
            TxControlFlow::ContinueProcessing(())
        } else {
            self.send_item(Err(RejectReason::SenderMustBeAdmin));
            TxControlFlow::IgnoreTx
        }
    }

    fn post_tx(
        &self,
        provisional_outcome: ProvisionalSequencerOutcome<S>,
        dirty_scratchpad: TxScratchpad<S, StateCheckpoint<S>>,
        slot_gas_meter_before_tx: &SlotGasMeter<S>,
        gas_used: &<S as Spec>::Gas,
    ) -> (StateCheckpoint<S>, TxControlFlow<TransactionReceipt<S>>) {
        let end_time: u64 = SystemTime::now().duration_since(UNIX_EPOCH).expect("SystemTime::now() returned something earlier than the UNIX epoch. This should be unreachable.").as_micros().try_into().expect("Unix time in micros overflowed u64. This should be unreachable for the next 300,000 years");
        let execution_time = end_time - self.unix_timestamp_micros.load(Ordering::SeqCst);
        let ProvisionalSequencerOutcome {
            reward,
            penalty,
            execution_status,
        } = provisional_outcome;
        let MaybeExecuted::Executed(receipt) = execution_status else {
            self.send_item(Err(RejectReason::SequencerOutOfGas));
            return (dirty_scratchpad.revert(), TxControlFlow::IgnoreTx);
        };

        if !receipt.receipt.is_successful() {
            let response = ExecutedTxResponse {
                receipt: receipt.clone(),
                tx_changes: dirty_scratchpad.tx_changes(),
                remaining_slot_gas: slot_gas_meter_before_tx
                    .remaining_preferred_slot_gas()
                    .clone(), // Since we ignore this tx, the remaining gas limit is unchanged
                execution_time_micros: execution_time,
            };

            self.send_item(Ok(response));
            return (dirty_scratchpad.revert(), TxControlFlow::IgnoreTx);
        }

        if penalty > reward || reward.saturating_sub(penalty) < self.tx_profit_threshold {
            self.send_item(Err(RejectReason::InsufficientReward {
                expected: self.tx_profit_threshold,
                found: reward.saturating_sub(penalty).0,
            }));
            return (dirty_scratchpad.revert(), TxControlFlow::IgnoreTx);
        }

        let remaining_slot_gas = slot_gas_meter_before_tx
            .remaining_preferred_slot_gas()
            .clone()
            .checked_sub(gas_used)
            // SAFETY: We always enforce that the gas used is less than the remaining slot gas limit
            .expect("Impossible happened: SlotGasMeter underflow when charging gas.");

        let response = ExecutedTxResponse {
            receipt: receipt.clone(),
            tx_changes: dirty_scratchpad.tx_changes(),
            remaining_slot_gas,
            execution_time_micros: execution_time,
        };

        self.send_item(Ok(response));
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
            MaybeAsyncBatch::Async { address, .. } => address.clone(),
            MaybeAsyncBatch::Sync { batch } => batch.sequencer_address.clone(),
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
            } => Handle::current().block_on(txs_receiver.recv()).map(|item| {
                (
                    item,
                    MaybeAsyncBatchControlFlow::Async(responder.clone_for_tx()),
                )
            }),
            MaybeAsyncBatch::Sync { batch } => batch
                .next()
                .map(|(item, _)| (item, MaybeAsyncBatchControlFlow::Sync)),
        }
    }
}
