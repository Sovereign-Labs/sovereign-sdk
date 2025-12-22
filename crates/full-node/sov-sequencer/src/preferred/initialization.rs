use crate::preferred::db::SequencerRole;

use super::*;
use anyhow::Context;
use anyhow::Result;
use sov_db::ledger_db::LedgerDb;
use sov_modules_api::rest::StateUpdateReceiver;
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::debug;

/// Builder for [`PreferredSequencer`] initialization.
pub struct Builder<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    da: Da,
    config: SequencerConfig<S::Address, PreferredSequencerConfig<S::Address>>,
    _phantom: PhantomData<(S, Rt)>,
}

impl<S, Rt, Da> Builder<S, Rt, Da>
where
    S: Spec,
    Rt: Runtime<S>,
    Da: DaService<Spec = S::Da>,
{
    /// Creates a new builder.
    pub fn new(
        da: Da,
        config: SequencerConfig<S::Address, PreferredSequencerConfig<S::Address>>,
    ) -> Self {
        Self {
            da,
            config,
            _phantom: PhantomData,
        }
    }

    /// Builds the sequencer instance.
    pub async fn build(
        self,
        state_update_receiver: StateUpdateReceiver<S::Storage>,
        storage_path: &Path,
        ledger_db: LedgerDb,
        api_ledger_db: LedgerDb,
        shutdown_sender: watch::Sender<()>,
        stop_at_rollup_height: Option<RollupHeight>,
    ) -> Result<(PreferredSequencer<S, Rt, Da>, Vec<JoinHandle<()>>)> {
        let shutdown_receiver = shutdown_sender.subscribe();
        let latest_state_update = state_update_receiver.borrow().clone();

        let da_address = self
            .da
            .get_signer()
            .await
            .context("Sequencer must have DaService configured with submit support")?;

        debug!(
            ?latest_state_update,
            %da_address,
            "Instantiating the preferred sequencer"
        );

        let mut config = self.config;
        let preferred_config = &config.sequencer_kind_config;

        let maybe_oracle_config =
            TimingOracleConfigWithPrivateKey::new(preferred_config.timing_oracle.clone())
                .transpose()?;
        maybe_add_oracle_to_admins(&mut config.admin_addresses, &maybe_oracle_config);

        let tx_status_manager = TxStatusManager::default();

        let (api_state, checkpoint_sender) = Self::api_state(latest_state_update.storage.clone());

        let (blobs_sender_channel, _) = broadcast::channel(preferred_config.events_channel_size);

        let (db, seq_role) = PreferredSequencerDb::new(
            shutdown_sender.clone(),
            preferred_config.is_replica,
            storage_path,
            &preferred_config.postgres_config,
        )
        .await?;

        let (next_sequence_number, db_cache) = db.initial_data().await?;
        let mut handles = vec![];

        let (blob_sender, blob_sender_handle) = PreferredBlobSender::new(
            self.da,
            ledger_db.clone(),
            db_cache.all_completed_blobs().clone(),
            storage_path.into(),
            tx_status_manager.clone(),
            shutdown_sender.clone(),
            Duration::from_secs(config.blob_processing_timeout_secs),
            blobs_sender_channel.clone(),
            seq_role,
        )
        .await?;

        if let Some(blob_sender_handle) = blob_sender_handle {
            handles.push(blob_sender_handle);
        }

        let (block_executors_shutdown_notifier, block_executors_shutdown_rx) = mpsc::channel(1);
        let (state_root_handle, state_root_task) = StateRootTask::create::<Rt>(
            block_executors_shutdown_rx,
            !preferred_config.disable_state_root_consistency_checks,
        );
        handles.push(state_root_handle);

        let cached_txs = TransactionCache::new(
            api_ledger_db.clone(),
            latest_state_update.next_tx_number,
            preferred_config.events_channel_size,
        );

        let (executor_events_sender, executor_events_receiver) =
            ExecutorEventsSender::new(shutdown_sender.clone(), db_cache);

        let in_flight_blobs = blob_sender.nb_of_in_flight_blobs();

        let rollup_exec_config = RollupBlockExecutorConfig {
            da_address,
            shutdown_notifier: block_executors_shutdown_notifier.clone(),
            state_root_request_sender: state_root_task.request_sender.clone(),
            shutdown_receiver: shutdown_receiver.clone(),
            shutdown_sender: shutdown_sender.clone(),
        };

        let (cache_warm_up_executor, workers) = CacheWarmUpExecutor::spawn_execution_task::<Rt>(
            latest_state_update.clone(),
            rollup_exec_config.clone(),
            config.clone(),
            seq_role,
        )
        .await;

        for worker in workers {
            handles.push(worker);
        }

        let (mut replica_task, start_replica_task_notifier) =
            ReplicaSyncTask::new(shutdown_sender.clone()).await?;

        let tx_queue_id = Arc::new(AtomicU64::new(0));
        let batch_execution_time_limit_micros =
            preferred_config.batch_execution_time_limit_millis * 1000;
        let (synchronized_state, synchronized_state_updator) = create(
            seq_role,
            api_ledger_db.clone(),
            latest_state_update.clone(),
            tx_queue_id.clone(),
            batch_execution_time_limit_micros,
            config.clone(),
            shutdown_receiver.clone(),
            shutdown_sender.clone(),
            executor_events_sender,
            next_sequence_number,
            in_flight_blobs,
            stop_at_rollup_height,
            rollup_exec_config.clone(),
            cached_txs.write_handle(),
            cache_warm_up_executor,
            start_replica_task_notifier,
        );

        let test_only_state_update_notification_receiver = synchronized_state
            .test_only_state_update_notification_sender
            .subscribe();
        let synchronized_state_task = synchronized_state.start().await;
        handles.push(synchronized_state_task);

        let side_effects_task = SideEffectsTask {
            checkpoint_sender,
            blob_sender,
            executor_events_receiver,
            db,
            shutdown_sender: shutdown_sender.clone(),
            transaction_cache: cached_txs.write_handle(),
        }
        .spawn();
        handles.push(side_effects_task);

        let synchronized_state_updator = Arc::new(synchronized_state_updator);
        let execution_backend = SequencerTxExecutionBackend {
            api_state: api_state.clone(),
            executor_queue_id: tx_queue_id.clone(),
            state_updator: synchronized_state_updator.clone(),
        };
        let (nonce_buffer_task, nonce_buffer_input) = NonceBufferTask::spawn(
            execution_backend,
            preferred_config.maximum_future_nonce_delta,
            preferred_config.future_nonce_transaction_timeout_millis,
            shutdown_receiver.clone(),
        );
        handles.push(nonce_buffer_task);

        let seq = PreferredSequencer(Arc::new(PreferredSequencerFields {
            synchronized_state_updator: synchronized_state_updator.clone(),
            tx_status_manager: tx_status_manager.clone(),
            transaction_cache: cached_txs,
            blobs_sender_channel: Some(blobs_sender_channel),
            api_state,
            _runtime: PhantomData,
            config: config.clone(),
            nonce_buffer_input,
            shutdown_receiver: shutdown_receiver.clone(),
            shutdown_sender: shutdown_sender.clone(),
            tx_queue_id,
            stop_at_rollup_height,
            test_only_state_update_notification_receiver,
            runtime: Rt::default(),
        }));

        // Launch replica sync task only for replicas.

        if let SequencerRole::Replica = seq_role {
            if let Some(postgres_config) = &preferred_config.postgres_config {
                let replica_task_handle = replica_task
                    .start(synchronized_state_updator, postgres_config)
                    .await;
                handles.push(replica_task_handle.data_fetcher_handle);
                handles.push(replica_task_handle.sync_task_handle);
            }
        }

        handles.push(tokio::spawn(update_state_task(
            seq.clone(),
            state_update_receiver.clone(),
            shutdown_receiver.clone(),
        )));

        handles.push(tokio::spawn({
            let ledger_db = ledger_db.clone();
            let seq = seq.clone();
            let shutdown_rx = shutdown_receiver.clone();
            async move {
                loop_send_tx_notifications::<S, Rt>(
                    state_update_receiver,
                    shutdown_rx,
                    &ledger_db,
                    seq.tx_status_manager(),
                )
                .await;
            }
        }));

        if let Some(oracle_config) = maybe_oracle_config {
            if let SequencerRole::Leader = seq_role {
                if Rt::default().maybe_set_oracle_timestamp(0).is_some() {
                    match update_timestamp_task(seq.clone(), oracle_config, shutdown_receiver) {
                        Ok(handle) => handles.push(handle),
                        Err(e) => {
                            error!(error = ?e, "Failed to start timestamp oracle task");
                        }
                    }
                }
            }
        }

        Ok((seq, handles))
    }

    fn api_state(
        storage: S::Storage,
    ) -> (
        ApiState<S>,
        watch::Sender<Arc<ConcurrentStateCheckpoint<S>>>,
    ) {
        let mut runtime: Rt = Default::default();
        assert!(
                accepts_preferred_batches(runtime.blob_selector()),
                "Attempting to use preferred sequencer with an incompatible rollup. Set your sequencer config to `standard` in your rollup's config.toml file or change your kernel to be compatible with soft confirmations."
            );
        let checkpoint = StateCheckpoint::new(storage, &runtime.kernel(), None);
        let concurrent_checkpoint = ConcurrentStateCheckpoint::from_state_checkpoint(checkpoint);
        let (checkpoint_sender, checkpoint_receiver) =
            watch::channel(Arc::new(concurrent_checkpoint));
        let api_state = ApiState::build(
            Arc::new(()),
            checkpoint_receiver,
            runtime.kernel_with_slot_mapping(),
            None,
        );
        (api_state, checkpoint_sender)
    }
}

fn maybe_add_oracle_to_admins<S: Spec>(
    admins: &mut Vec<S::Address>,
    oracle_config: &Option<TimingOracleConfigWithPrivateKey<S>>,
) {
    if let Some(oracle_config) = oracle_config {
        let oracle = oracle_config.address();
        if !admins.contains(&oracle) {
            info!(
                "Adding oracle address {} to sequencer's admin address list",
                oracle
            );
            admins.push(oracle);
        }
    };
}
