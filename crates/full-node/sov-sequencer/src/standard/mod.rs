//! Standard, "vanilla" non-preferred sequencer implementation.

mod mempool;

use std::boxed::Box;
use std::marker::PhantomData;
use std::num::NonZero;
use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use axum::http::StatusCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sov_blob_sender::{new_blob_id, BlobSender};
use sov_db::ledger_db::LedgerDb;
use sov_modules_api::capabilities::{AuthenticationError, ChainState};
use sov_modules_api::rest::utils::ErrorObject;
use sov_modules_api::rest::{ApiState, StateUpdateReceiver};
use sov_modules_api::transaction::SequencerReward;
use sov_modules_api::*;
use sov_modules_stf_blueprint::{process_tx_and_reward_prover, ApplyTxResult, PreExecError};
use sov_rest_utils::json_obj;
use sov_rollup_interface::node::da::DaService;
use sov_rollup_interface::node::DaSyncState;
use thiserror::Error;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinHandle;
use tokio::time::Duration;
use tracing::{debug, error, trace, warn};

use self::mempool::{Mempool, MempoolCursor, MempoolTx};
use crate::common::{
    loop_call_update_state, loop_send_tx_notifications, pre_exec_err_to_accept_tx_err,
    sender_is_allowed, tx_auth, AcceptedTx, EmptyConfirmation, Sequencer, TxStatusBlobSenderHooks,
    WithCachedTxHashes,
};
use crate::{
    ProofBlobSender, SequencerConfig, SequencerNotReadyDetails, TxHash, TxStatus, TxStatusManager,
};

struct Inner<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    assembled_batch: Option<WithCachedTxHashes<Vec<FullyBakedTx>>>,
    blob_sender: BlobSender<Da, TxStatusBlobSenderHooks<Da::Spec>, LedgerDb>,
    checkpoint: Option<StateCheckpoint<S>>,
    mempool: Mempool<Da::Spec>,
    phantom: PhantomData<Rt>, // TODO(@neysofu): remove this if possible.
}

/// Configuration for [`StdSequencer`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct StdSequencerConfig {
    /// Maximum number of transactions in mempool. Once this limit is reached,
    /// the batch builder will evict older transactions.
    pub mempool_max_txs_count: Option<NonZero<usize>>,
    /// Maximum size of a batch. The sequencer will not build batches larger
    /// than this size.
    pub max_batch_size_bytes: Option<NonZero<usize>>,
}

/// A [`Sequencer`] that creates batches of transactions in a way that's
/// reasonably "fair" to everybody.
///
/// Transactions are included in batches by following a largest-first,
/// least-recent-first priority. Only transactions that were successfully
/// dispatched are included.
pub struct StdSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    runtime: Rt,
    txsm: TxStatusManager<S::Da>,
    inner: Mutex<Inner<S, Rt, Da>>,
    checkpoint_sender: watch::Sender<StateCheckpoint<S>>,
    api_state: ApiState<S>,
    da_address: <S::Da as DaSpec>::Address,
    config: SequencerConfig<S::Address, StdSequencerConfig>,
    api_ledger_db: LedgerDb,
}

/// An error that indicates that the transaction could not be added to the batch.
#[derive(Debug, Error)]
enum AddTxToBatchError {
    /// Error occurred during transaction processing.
    #[error("Error occurred during transaction processing")]
    TxProcessing(#[source] TxProcessingError),
    /// Error occurred during pre-execution checks.
    #[error("Error occurred during pre-execution checks")]
    PreExecCheck(#[source] PreExecError),
    /// The transaction was rejected because it is not sequencer safe.
    #[error("The transaction was rejected because the it invokes the `{0}` module which may modify sequencer configurations. You need admin permissions on the sequencer to call this method.")]
    PermissionDenied(&'static str),
}

impl<S, Rt, Da> StdSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    /// Creates `StdSequencer`
    #[allow(clippy::too_many_arguments)]
    pub async fn create(
        da: Da,
        state_update_receiver: StateUpdateReceiver<S::Storage>,
        _da_sync_state: Arc<DaSyncState>,
        storage_path: &Path,
        config: &SequencerConfig<S::Address, StdSequencerConfig>,
        ledger_db: LedgerDb,
        api_ledger_db: LedgerDb,
        shutdown_sender: watch::Sender<()>,
    ) -> anyhow::Result<(Arc<Self>, Vec<JoinHandle<()>>)> {
        let shutdown_receiver = shutdown_sender.subscribe();
        let mut runtime = Rt::default();
        let kernel_with_slot_mapping = runtime.kernel_with_slot_mapping();

        let latest_state_update = state_update_receiver.borrow().clone();
        let checkpoint =
            StateCheckpoint::new(latest_state_update.storage.clone(), &runtime.kernel());
        let (checkpoint_sender, checkpoint_receiver) = watch::channel(checkpoint);

        let api_state = ApiState::build(
            Arc::new(()),
            checkpoint_receiver,
            kernel_with_slot_mapping,
            None,
        );

        let txsm = TxStatusManager::default();
        let checkpoint =
            StateCheckpoint::new(latest_state_update.storage.clone(), &runtime.kernel());

        let da_address = da.get_signer().await;
        let (blob_sender, blob_sender_handle) = BlobSender::new(
            da,
            ledger_db.clone(),
            storage_path,
            TxStatusBlobSenderHooks::new(txsm.clone()),
            shutdown_sender,
            Duration::from_secs(config.blob_processing_timeout_secs),
            None,
            Default::default(),
        )
        .await?;

        let mut handles: Vec<JoinHandle<()>> = vec![];
        handles.push(blob_sender_handle);

        let inner = Inner {
            blob_sender,
            checkpoint: Some(checkpoint),
            assembled_batch: None,
            phantom: PhantomData,
            mempool: Mempool::new(
                txsm.clone(),
                config
                    .sequencer_kind_config
                    .mempool_max_txs_count
                    .unwrap_or(default_mempool_max_txs_count()),
            )?,
        };

        let seq = Arc::new(StdSequencer {
            inner: inner.into(),
            txsm,
            api_state,
            runtime: Rt::default(),
            checkpoint_sender,
            config: config.clone(),
            api_ledger_db,
            da_address,
        });

        handles.push(tokio::spawn({
            loop_call_update_state(
                seq.clone(),
                state_update_receiver.clone(),
                shutdown_receiver.clone(),
            )
        }));
        handles.push(tokio::spawn({
            let ledger_db = ledger_db.clone();
            let seq = seq.clone();
            async move {
                loop_send_tx_notifications::<S, Rt>(
                    state_update_receiver,
                    shutdown_receiver,
                    &ledger_db,
                    seq.tx_status_manager(),
                )
                .await;
            }
        }));

        Ok((seq, handles))
    }

    /// Returns [`None`] if the transaction does not fit inside the batch.
    #[allow(clippy::type_complexity)]
    fn try_add_tx_to_batch(
        &self,
        mempool_tx: &MempoolTx,
        mut ctx: BatchConstructionContext<S>,
    ) -> (
        BatchConstructionContext<S>,
        Result<Option<(FullyBakedTx, TransactionReceipt<S>)>, AddTxToBatchError>,
    ) {
        let tx = mempool_tx.tx.clone();
        let mut runtime = Rt::default();

        let operating_mode = runtime
            .chain_state()
            .operating_mode(&mut ctx.state_checkpoint);

        // To fill a batch as big as possible, we only check if valid
        // tx can fit in the batch.
        let tx_len = tx.data.len();
        if ctx.current_batch_size_in_bytes + tx_len > self.max_batch_size_bytes().get() {
            return (ctx, Ok(None));
        }

        let tx_scratchpad = ctx.state_checkpoint.to_tx_scratchpad();

        let (tx_scratchpad, output_res) =
            tx_auth::<S, Rt, _>(tx_scratchpad, ctx.gas_price.clone(), &mempool_tx.tx);

        let (auth_output, gas_meter) = match output_res {
            Ok(ok) => ok,

            Err(err) => {
                ctx.state_checkpoint = tx_scratchpad.revert();
                return (ctx, Err(AddTxToBatchError::PreExecCheck(err)));
            }
        };

        let (_, authz_data, message) = &auth_output;

        // If the module isn't sequencer safe, the caller must be an admin to proceed
        if !sender_is_allowed(
            &self.runtime,
            message,
            &authz_data.default_address,
            &self.da_address,
            &self.config.admin_addresses,
        ) {
            ctx.state_checkpoint = tx_scratchpad.revert();
            return (
                ctx,
                Err(AddTxToBatchError::PermissionDenied(
                    message.discriminant().into(),
                )),
            );
        }

        let pre_exec_working_set = tx_scratchpad.to_pre_exec_working_set(gas_meter);
        let (res, tx_scratchpad, _gas_meter) = process_tx_and_reward_prover(
            &mut runtime,
            pre_exec_working_set,
            // Currently the sequencer doesn't take into account the slot gas limit.
            &<S::Gas>::MAX,
            auth_output,
            mempool_tx.tx.clone(),
            &self.da_address,
            self.config.rollup_address.clone(),
            ExecutionContext::Sequencer,
            &NoOpControlFlow,
            operating_mode,
        );

        match res {
            Err(reason) => {
                // ...and immediately store the new `StateCheckpoint`.
                ctx.state_checkpoint = tx_scratchpad.revert();
                (ctx, Err(AddTxToBatchError::TxProcessing(reason.0)))
            }
            Ok(ApplyTxResult {
                receipt,
                transaction_consumption,
            }) => {
                let sequencer_reward = transaction_consumption.priority_fee();
                // ...and immediately store the new `StateCheckpoint`.
                ctx.state_checkpoint = tx_scratchpad.commit();
                ctx.reward.accumulate(sequencer_reward);

                (ctx, Ok(Some((tx, receipt))))
            }
        }
    }

    async fn produce_batch(&self) -> anyhow::Result<Option<WithCachedTxHashes<Vec<FullyBakedTx>>>> {
        tracing::debug!("`produce_batch` has been called");
        let mut inner = self.inner.lock().await;

        // We already have a batch assembled. We'll wait until it's popped
        // before assembling a new one.
        if inner.assembled_batch.is_some() {
            return Ok(None);
        }

        let state_checkpoint = inner.checkpoint.take().unwrap();
        let mempool = &mut inner.mempool;

        // This closure helps us make sure that we always put the
        // `StateCheckpoint` back into `self` at the end of the function.
        let (new_checkpoint, response) = (|mut checkpoint| {
            let mut runtime: Rt = Default::default();
            let gas_price = runtime.chain_state().base_fee_per_gas(&mut checkpoint).expect("Impossible to get the gas price for the current slot. This is a bug. Please report it");

            let mut ctx = BatchConstructionContext {
                reward: SequencerReward::ZERO,
                gas_price,
                state_checkpoint: checkpoint,
                current_batch_size_in_bytes: 0,
            };

            let mut txs = Vec::new();

            let count_before = mempool.len();
            tracing::debug!(
                txs_count = count_before,
                "Going to build batch from transactions in mempool"
            );

            let mut cursor = self.mempool_cursor(&ctx);

            while let Some(mempool_tx) = mempool.next(&mut cursor) {
                let (context, tx_receipt) = self.try_add_tx_to_batch(&mempool_tx, ctx);
                ctx = context;

                let tx_receipt = match tx_receipt {
                    Ok(txr) => txr,
                    Err(add_tx_error) => match add_tx_error {
                        AddTxToBatchError::PreExecCheck(pre_exec_error) => match pre_exec_error {
                            // If authentication is fatally broken, we can
                            // never submit it succesfully so drop it from the mempool
                            PreExecError::AuthError(AuthenticationError::FatalError(_, _)) => {
                                tracing::info!(hash= %mempool_tx.hash, error = %pre_exec_error, "Invalid tx detected in mempool; dropping tx",);
                                mempool.drop_and_notify(
                                    &mempool_tx.hash,
                                    "Transaction is invalid".to_string(),
                                );
                                continue;
                            }
                            PreExecError::SequencerError(_)
                            | PreExecError::AuthError(AuthenticationError::OutOfGas(_)) => {
                                tracing::info!(hash= %mempool_tx.hash, reason = %pre_exec_error, "The current state of the rollup didn't allow tx inclusion; ignoring tx",);
                                continue;
                            }
                        },
                        AddTxToBatchError::TxProcessing(tx_processing_error) => {
                            tracing::info!(hash= %mempool_tx.hash, reason = %tx_processing_error, "The current state of the rollup didn't allow tx inclusion; ignoring tx",);
                            continue;
                        }
                        AddTxToBatchError::PermissionDenied(module) => {
                            mempool.drop_and_notify(&mempool_tx.hash, add_tx_error.to_string());
                            tracing::info!(hash= %mempool_tx.hash, target_module = %module, "Tx attempted to invoke a sequencer-unsafe module without appropriate permissions; dropping tx");
                            continue;
                        }
                    },
                };

                match tx_receipt.map(|(tx, r)| (tx, r.receipt)) {
                    Some((fully_baked_tx, TxEffect::Successful(_))) => {
                        tracing::info!(
                            hash = %mempool_tx.hash,
                            "Transaction has been included in the batch",
                        );

                        ctx.current_batch_size_in_bytes += fully_baked_tx.data.len();

                        txs.push(TxWithHash {
                            fully_baked_tx,
                            hash: mempool_tx.hash,
                        });

                        // Update the cursor to reflect the new amount of available
                        // space inside the batch.
                        cursor = cursor.max(self.mempool_cursor(&ctx));
                    }
                    Some((fully_baked_tx, tx_receipt)) => {
                        // Failed transaction; ignore and process the next one.
                        tracing::warn!(
                            ?tx_receipt,
                            tx = hex::encode(&fully_baked_tx),
                            hash = %mempool_tx.hash,
                            "Error during transaction dispatch"
                        );
                        continue;
                    }
                    None => {
                        // We couldn't find any transaction that fits in the
                        // remaining space inside the batch; we're done.
                        break;
                    }
                }
            }

            if txs.is_empty() {
                return (ctx.state_checkpoint, Ok(None));
            }

            trace!(
                txs_count = txs.len(),
                "Batch of transactions has been built"
            );

            let (txs, tx_hashes): (Vec<_>, Vec<_>) = txs
                .into_iter()
                .map(|tx| (tx.fully_baked_tx, tx.hash))
                .unzip();

            (
                ctx.state_checkpoint,
                Ok(Some(WithCachedTxHashes {
                    inner: txs,
                    tx_hashes: tx_hashes.into(),
                })),
            )
        })(state_checkpoint);

        inner.checkpoint = Some(new_checkpoint);

        match response {
            Ok(Some(batch)) => {
                inner.assembled_batch = Some(batch.clone());

                for tx_hash in &*batch.tx_hashes {
                    // We're not dropping transactions because we're evicting them,
                    // but rather because we don't need them anymore after
                    // submitting them. Thus, sending a "drop" notification to users
                    // would be semantically wrong.
                    inner.mempool.drop_without_notifying(tx_hash);
                }

                Ok(Some(batch))
            }
            Ok(None) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn mempool_cursor(&self, ctx: &BatchConstructionContext<S>) -> MempoolCursor {
        MempoolCursor::new(
            self.max_batch_size_bytes()
                .get()
                .saturating_sub(ctx.current_batch_size_in_bytes),
        )
    }

    fn max_batch_size_bytes(&self) -> NonZero<usize> {
        self.config
            .sequencer_kind_config
            .max_batch_size_bytes
            .unwrap_or(self.default_max_batch_size_bytes())
    }

    fn default_max_batch_size_bytes(&self) -> NonZero<usize> {
        // 1 MiB
        NonZero::new(1024 * 1024).unwrap()
    }

    async fn publish_batch(
        &self,
        batch: &WithCachedTxHashes<Vec<FullyBakedTx>>,
    ) -> anyhow::Result<()> {
        let serialized_batch = borsh::to_vec::<Vec<FullyBakedTx>>(&batch.inner)?.into();
        let blob_id = new_blob_id();

        let mut inner = self.inner.lock().await;

        inner
            .blob_sender
            .hooks()
            .add_txs(blob_id, batch.tx_hashes.clone())
            .await;
        inner
            .blob_sender
            .publish_batch_blob(serialized_batch, blob_id)
            .await?;

        Ok(())
    }
}

#[cfg(feature = "test-utils")]
#[allow(missing_docs)]
impl<S, Rt, Da> StdSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    pub async fn produce_and_submit_batch(&self) -> Option<WithCachedTxHashes<Vec<FullyBakedTx>>> {
        match self.produce_batch().await {
            Ok(Some(batch)) => {
                self.publish_batch(&batch).await.unwrap();
                Some(batch)
            }
            _ => None,
        }
    }
}

#[async_trait]
impl<S, Rt, Da> Sequencer for StdSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    // The standard, non-preferred sequencer doesn't provide any information as
    // part of transaction confirmations. In the future, it might return
    // authentication gas usage information.
    type Confirmation = EmptyConfirmation;
    type Spec = S;
    type Rt = Rt;
    type Da = Da;

    async fn is_ready(&self) -> Result<(), SequencerNotReadyDetails> {
        // The non-preferred batch builder is always ready to accept
        // transactions.
        Ok(())
    }

    fn tx_status_manager(&self) -> &TxStatusManager<<Self::Spec as Spec>::Da> {
        &self.txsm
    }

    fn api_state(&self) -> ApiState<Self::Spec> {
        self.api_state.clone()
    }

    async fn update_state(
        &self,
        state_update_info: StateUpdateInfo<S::Storage>,
    ) -> anyhow::Result<()> {
        let StateUpdateInfo {
            storage,
            slot_number,
            ledger_reader,
            ..
        } = &state_update_info;
        let checkpoint = StateCheckpoint::new(storage.clone(), &Rt::default().kernel());

        tracing::debug!(
            %slot_number,
            "The sequencer received a new state. Notifying the subscribers."
        );

        {
            let mut inner = self.inner.lock().await;
            self.checkpoint_sender
                .send(checkpoint.clone_with_empty_witness_dropping_temp_cache())
                .ok();
            inner.checkpoint = Some(checkpoint);
        }

        self.api_ledger_db.replace_reader(ledger_reader.clone());
        self.api_ledger_db.send_notifications_for_slot(*slot_number);

        if self.config.automatic_batch_production {
            match self.produce_batch().await {
                Ok(Some(batch)) => {
                    self.publish_batch(&batch).await?;
                }
                Ok(None) => {}
                Err(e) => {
                    warn!(error = %e, %slot_number, "Couldn't produce a batch at this time (possibly due to imminent shutdown), continuing");
                }
            }
        }

        Ok(())
    }

    async fn accept_tx(
        &self,
        baked_tx: FullyBakedTx,
    ) -> Result<AcceptedTx<Self::Confirmation>, ErrorObject> {
        tracing::trace!(
            baked_tx = hex::encode(&baked_tx),
            "`accept_tx` has been called"
        );

        if baked_tx.data.len() > self.max_batch_size_bytes().get() {
            return Err(ErrorObject {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: "Transaction is too big".to_string(),
                details: json_obj!({
                    "max_allowed_size": self.max_batch_size_bytes(),
                    "submitted_size": baked_tx.data.len(),
                }),
            });
        }

        let mut inner = self.inner.lock().await;
        let state_checkpoint = inner
            .checkpoint
            .take()
            .expect("Absent checkpoint; this is a bug, please report it");

        // This closure helps us make sure that we always put the
        // state checkpoint back into `self` at the end of the function.
        let (new_checkpoint, response) = (|mut checkpoint: StateCheckpoint<S>| {
            let mut runtime: Rt = Default::default();
            let gas_price = runtime.chain_state().base_fee_per_gas(&mut checkpoint).expect("Impossible to get the gas price for the current slot. This is a bug. Please report it");

            let (tx_scratchpad, output_res) =
                tx_auth::<S, Rt, _>(checkpoint.to_tx_scratchpad(), gas_price, &baked_tx.clone());

            let (auth_output, gas_meter) = match output_res {
                Ok(ok) => ok,
                Err(error) => {
                    return (
                        tx_scratchpad.revert(),
                        Err(pre_exec_err_to_accept_tx_err(error)),
                    );
                }
            };

            let tx_hash = auth_output.0.raw_tx_hash;

            let gas_info = gas_meter.gas_info();
            let tx = auth_output.0.authenticated_tx;

            let working_set_gas_meter = tx.gas_meter(&gas_info.gas_price.clone(), &<S::Gas>::MAX);

            let mut working_set =
                WorkingSet::create_working_set(tx_scratchpad, &tx, working_set_gas_meter);

            if let Err(err) = working_set.charge_gas(&gas_info.gas_used) {
                let (scratchpad, _) = working_set.revert();

                return (
                    scratchpad.revert(),
                    Err(ErrorObject {
                        // Not enough gas, so 403 seems appropriate.
                        status: StatusCode::FORBIDDEN,
                        message: "Not enough gas for pre-execution checks".to_string(),
                        details: json_obj!({
                            "error": err.to_string()
                        }),
                    }),
                );
            };

            (working_set.finalize().0.commit(), Ok(tx_hash))
        })(state_checkpoint);

        let tx_hash = response?;
        {
            inner.mempool.add_new_tx(tx_hash, baked_tx.clone());
            tracing::trace!(
                %tx_hash,
                "Transaction has been added to the mempool"
            );
        }

        inner.checkpoint = Some(new_checkpoint);

        self.tx_status_manager()
            .notify(tx_hash, TxStatus::Submitted);

        Ok(AcceptedTx {
            tx: baked_tx,
            tx_hash,
            confirmation: EmptyConfirmation {},
        })
    }

    async fn tx_status(
        &self,
        tx_hash: &TxHash,
    ) -> anyhow::Result<
        TxStatus<<<Self::Spec as Spec>::Da as sov_modules_api::DaSpec>::TransactionId>,
    > {
        if let Some(status) = self.txsm.get_cached(tx_hash) {
            return Ok(status);
        }

        let inner = self.inner.lock().await;

        if inner.mempool.contains(tx_hash) {
            Ok(TxStatus::Submitted)
        } else {
            Ok(TxStatus::Unknown)
        }
    }
}

#[async_trait]
impl<S, Rt, Da> ProofBlobSender for StdSequencer<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    async fn produce_and_publish_proof_blob(&self, proof_blob: Arc<[u8]>) -> anyhow::Result<()> {
        let blob_id = new_blob_id();

        // TODO: Put SerializedAggregatedProof directly on chain without
        // wrapping in a vec
        // <https://github.com/Sovereign-Labs/sovereign-sdk-wip/issues/1065>
        let blob_bytes = borsh::to_vec(&proof_blob)?.into();

        debug!(blob_id, "Dispatching proof blob for publishing");

        self.inner
            .lock()
            .await
            .blob_sender
            .publish_proof_blob(blob_bytes, blob_id)
            .await?;

        Ok(())
    }
}

struct BatchConstructionContext<S: Spec> {
    state_checkpoint: StateCheckpoint<S>,
    reward: SequencerReward,
    gas_price: <S::Gas as Gas>::Price,
    current_batch_size_in_bytes: usize,
}

const fn default_mempool_max_txs_count() -> NonZero<usize> {
    match NonZero::new(100) {
        Some(default) => default,
        None => panic!("100 is greater than 0"),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TxWithHash {
    fully_baked_tx: FullyBakedTx,
    hash: TxHash,
}
