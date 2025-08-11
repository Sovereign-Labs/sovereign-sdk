//! Implementation for the [`MetricsTracker`] for Influxdb.

use std::fmt::Debug;
use std::io::Write;
use std::sync::OnceLock;

use sov_rollup_interface::common::{HexHash, VisibleSlotNumber};

use crate::influxdb::{publisher, safe_telegraf_string, Metric};
use crate::{MaybeTimer, MetricsTracker, MonitoringConfig};

pub(crate) static METRICS_TRACKER: OnceLock<MetricsTracker> = OnceLock::new();

/// Alias for number of nano-seconds since unix epoch.
pub(crate) type Timestamp = u128;

/// Spawns task that published metrics in the background.
pub fn init_metrics_tracker(config: &MonitoringConfig) {
    if METRICS_TRACKER.get().is_none() {
        let (sender, receiver) =
            tokio::sync::mpsc::channel(config.get_max_pending_metrics() as usize);
        let config = config.clone();
        let _handle = tokio::spawn(async move {
            publisher::metrics_publisher_task(receiver, &config).await;
        });
        tracing::trace!("Metrics tracker initialized");
        OnceLock::set(&METRICS_TRACKER, MetricsTracker { sender })
            .expect("Metrics tracker failed to set metrics");
    }
}

#[derive(Debug)]
struct MetricWithTimestamp(Box<dyn Metric>, Timestamp);

impl Metric for MetricWithTimestamp {
    fn measurement_name(&self) -> &'static str {
        self.0.measurement_name()
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        self.0.serialize_for_telegraf(buffer)?;
        write!(buffer, " {}", self.1)
    }

    #[cfg(feature = "gas-constant-estimation")]
    fn write_to_csv(&self, writers: &mut super::csv_helper::CsvWriters) -> std::io::Result<()> {
        self.0.write_to_csv(writers)
    }
}

impl MetricsTracker {
    /// Quick way to submit a string metric without dealing with [`Metric`].
    pub fn submit_inline(&self, measurement: &'static str, rest: impl ToString) {
        #[derive(Debug)]
        struct InlineMetric(&'static str, String);

        impl Metric for InlineMetric {
            fn measurement_name(&self) -> &'static str {
                self.0
            }

            fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
                write!(buffer, "{} {}", self.measurement_name(), self.1)
            }
        }

        self.submit(InlineMetric(measurement, rest.to_string()));
    }

    /// Submits a metric.
    pub fn submit(&self, measurement: impl Metric + 'static) {
        self.submit_with_time(timestamp(), measurement);
    }

    /// Submits a metric with a timestamp.
    pub fn submit_with_time(&self, timestamp: u128, measurement: impl Metric + 'static) {
        tracing::trace!(?measurement, "Submitting a measurement");
        let metric = MetricWithTimestamp(Box::new(measurement), timestamp);

        if let Err(e) = self.sender.try_send(Box::new(metric)) {
            tracing::trace!(error = ?e, "Dropped measurement");
        };
    }

    /// Tracks all runner-related metrics
    pub fn track_runner_metrics(&self, point: RunnerMetrics) {
        let timestamp = timestamp();
        let RunnerMetrics {
            da_height: da_height_processed,
            sync_distance,
            get_block_time,
            batches_processed,
            batch_bytes_processed,
            transactions_processed,
            proofs_processed,
            proof_bytes_processed,
            process_slot_time,
            apply_slot_time,
            stf_transition_time,
            extract_blobs_time,
            extraction_proof_time,
            processing_changes_time,
        } = point;
        self.submit_with_time(
            timestamp,
            RunnerDaMetrics {
                da_height: da_height_processed,
                sync_distance,
                get_block_time,
            },
        );
        self.submit_with_time(
            timestamp,
            RunnerCountMetrics {
                da_height: da_height_processed,
                batches: batches_processed,
                batch_bytes: batch_bytes_processed,
                transactions: transactions_processed,
                proofs_processed,
                proof_bytes_processed,
            },
        );
        self.submit_with_time(
            timestamp,
            RunnerTimeMetrics {
                da_height: da_height_processed,
                process_slot_time,
                apply_slot_time,
                stf_transition_time,
                extract_blobs_time,
                extraction_proof_time,
                processing_changes_time,
            },
        );
    }
}

/// Returns the current timestamp in nanoseconds since the UNIX epoch.
pub fn timestamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

/// Metrics related to the main loop of STF runner.
pub struct RunnerMetrics {
    /// DA height processed in this iteration.
    pub da_height: u64,
    /// Distance between processed DA height and DA head.
    pub sync_distance: i64,
    /// Time it took to fetch given block from DA layer.
    pub get_block_time: std::time::Duration,
    /// Number of batches with transactions processed in this iteration.
    pub batches_processed: u64,
    /// Total size of blobs with transactions being processed.
    pub batch_bytes_processed: u64,
    /// Total number of transactions being processed
    pub transactions_processed: u64,
    /// Total number of proofs being processed in this iteration.
    pub proofs_processed: u64,
    /// Total size of proofs.
    pub proof_bytes_processed: u64,
    /// Full time it took to process slot.
    /// Includes all operations required together with pre/post-processing.
    /// process_slot_time == (get_block_time + extract_blobs_time + extraction_proof_time + stf_transition_time)
    pub process_slot_time: std::time::Duration,
    /// Time it took to execute only STF transition without post-processing or committing to the DB.
    pub apply_slot_time: std::time::Duration,
    /// Time it took to execute the STF transition, post-process, and commit all results to the DB,
    /// but without fetching data from DA and extracting it.
    pub stf_transition_time: std::time::Duration,
    /// Time it took to extract relevant blobs from the whole DA block.
    pub extract_blobs_time: std::time::Duration,
    /// Time it took to build proof that the relevant blobs were extracted correctly.
    pub extraction_proof_time: std::time::Duration,
    /// Time it took to process changes after slot has been applied proofs has been processed.
    pub processing_changes_time: std::time::Duration,
}

#[derive(Debug)]
pub(crate) struct RunnerDaMetrics {
    pub da_height: u64,
    pub sync_distance: i64,
    pub get_block_time: std::time::Duration,
}

#[derive(Debug)]
pub(crate) struct RunnerCountMetrics {
    pub da_height: u64,
    pub batches: u64,
    pub batch_bytes: u64,
    pub transactions: u64,
    pub proofs_processed: u64,
    pub proof_bytes_processed: u64,
}

#[derive(Debug)]
pub(crate) struct RunnerTimeMetrics {
    pub da_height: u64,
    pub process_slot_time: std::time::Duration,
    pub apply_slot_time: std::time::Duration,
    pub stf_transition_time: std::time::Duration,
    pub extract_blobs_time: std::time::Duration,
    pub extraction_proof_time: std::time::Duration,
    pub processing_changes_time: std::time::Duration,
}

/// Detailed metrics on how much time it took to process changes after a slot has been applied.
#[derive(Debug)]
pub struct RunnerProcessStfChangesMetrics {
    /// DA height at which changes were processed.
    pub da_height: u64,
    /// Number of aggregated proofs were to process.
    pub aggregated_proofs_count: usize,
    /// A number of transitions have to be finalized.
    pub finalized_transitions_count: usize,
    /// Time the whole operation took.
    pub total_time: std::time::Duration,
    /// Time it took to identify which of the seen transitions can be considered as finalized.
    pub processing_finalized_transitions_time: std::time::Duration,
    /// Time it took to materialize all ledger-related changes.
    pub ledger_changes_materializing_time: std::time::Duration,
    /// Time to save changes to the storage manager.
    pub saving_to_storage_time: std::time::Duration,
    /// Time to commit all finalized block headers in the storage manager.
    pub committing_storage_time: std::time::Duration,
    /// Time it took to send updated API and LedgerDB storage.
    pub updating_api_storage_time: std::time::Duration,
    /// Time it took to send STF info to the prover, if prover is enabled.
    pub sending_stf_info_time_to_prover_time: std::time::Duration,
}

/// Simplified version of [`sov_rollup_interface::stf::TxEffect`]
#[derive(Debug)]
pub enum TransactionEffect {
    /// The transaction was skipped.
    Skipped,
    /// The transaction was reverted during execution.
    Reverted,
    /// The transaction was processed successfully.
    Successful,
}

impl<T: sov_rollup_interface::stf::TxReceiptContents> From<&sov_rollup_interface::stf::TxEffect<T>>
    for TransactionEffect
{
    fn from(value: &sov_rollup_interface::stf::TxEffect<T>) -> Self {
        match value {
            sov_rollup_interface::stf::TxEffect::Skipped(_) => TransactionEffect::Skipped,
            sov_rollup_interface::stf::TxEffect::Reverted(_) => TransactionEffect::Reverted,
            sov_rollup_interface::stf::TxEffect::Successful(_) => TransactionEffect::Successful,
        }
    }
}

/// Collection of metrics related to transaction processing.
#[derive(Debug)]
pub struct TransactionProcessingMetrics {
    /// Time it took a transaction to be executed
    pub execution_time: std::time::Duration,
    /// The effect of the transaction,
    /// simplified version of [`sov_rollup_interface::stf::TxEffect`]
    pub tx_effect: TransactionEffect,
    /// [`sov_rollup_interface::stf::ExecutionContext`]
    pub execution_context: sov_rollup_interface::stf::ExecutionContext,
    /// Height at which transaction is being executed
    pub visible_slot_number: VisibleSlotNumber,
    /// Human-readable address of sequencer.
    pub sequencer_address: String,
    /// Call message
    pub call_message: String,
    /// Gas used by given transaction expressed in gas units
    pub gas_used: Vec<u64>,
}

/// Metrics related to processing of a single slot.
#[derive(Debug)]
pub struct SlotProcessingMetrics {
    /// Time it took from slot initialization till blobs have been selected.
    /// Includes kernel and state initialization + chain_state logic.
    pub blobs_selection_time: std::time::Duration,

    /// Time it took to materialize slot changes and finalize slot hooks.
    pub slot_finalization_time: std::time::Duration,

    /// Height of DA layer when this slot has been applied.
    pub da_height: u64,

    /// Visible slot number at given slot.
    pub visible_slot_number: VisibleSlotNumber,

    /// [`sov_rollup_interface::stf::ExecutionContext`]
    pub execution_context: sov_rollup_interface::stf::ExecutionContext,

    /// Gas used during slot processing, expressed in gas units
    pub gas_used: Vec<u64>,
}

/// Metrics related to processing of a single slot.
#[derive(Debug)]
pub struct UserSpaceSlotProcessingMetrics {
    /// Time taken by begin_rollup_block hook.
    pub begin_block_hook_time: std::time::Duration,

    /// Time it took to process all blobs: Batches, Proofs and Forced registration
    pub blobs_processing_time: std::time::Duration,

    /// Time taken by end_rollup_block hook.
    pub end_block_hook_time: std::time::Duration,

    /// The visible slot number associated with these metrics.
    pub visible_slot_number: VisibleSlotNumber,

    /// [`sov_rollup_interface::stf::ExecutionContext`]
    pub execution_context: sov_rollup_interface::stf::ExecutionContext,

    /// Gas used during slot processing, expressed in gas units
    pub gas_used: Vec<u64>,
}

impl Metric for RunnerDaMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_runner_da"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} da_height={},sync_distance={},get_block_time_ms={}",
            self.measurement_name(),
            self.da_height,
            self.sync_distance,
            self.get_block_time.as_millis(),
        )
    }
}

impl Metric for RunnerCountMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_runner_counts"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} da_height={},batches_c={},transactions_c={},proofs_c={},batch_bytes={},proof_bytes={}",
            self.measurement_name(),
            self.da_height,
            self.batches,
            self.transactions,
            self.proofs_processed,
            self.batch_bytes,
            self.proof_bytes_processed,
        )
    }
}

impl Metric for RunnerTimeMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_runner_times_us"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} da_height={},process_slot={},apply_slot={},stf_transition={},extract_blobs={},blob_extraction_proof={},processing_changes_time={}",
            self.measurement_name(),
            self.da_height,
            self.process_slot_time.as_micros(),
            self.apply_slot_time.as_micros(),
            self.stf_transition_time.as_micros(),
            self.extract_blobs_time.as_micros(),
            self.extraction_proof_time.as_micros(),
            self.processing_changes_time.as_micros(),
        )
    }
}

impl Metric for RunnerProcessStfChangesMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_runner_process_stf_changes"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},da_height={} proofs_count={},finalized_transitions_count={},total_time_us={},processing_finalized_transitions_time_us={},ledger_materializing_time_us={},saving_to_storage_time={},committing_storage_time={},update_api_storage_time={},sending_stf_to_prover_time={}",
            self.measurement_name(),
            // Tags
            self.da_height,
            // Fields
            self.aggregated_proofs_count,
            self.finalized_transitions_count,
            self.total_time.as_micros(),
            self.processing_finalized_transitions_time.as_micros(),
            self.ledger_changes_materializing_time.as_micros(),
            self.saving_to_storage_time.as_micros(),
            self.committing_storage_time.as_micros(),
            self.updating_api_storage_time.as_micros(),
            self.sending_stf_info_time_to_prover_time.as_micros(),
        )
    }
}

impl Metric for TransactionProcessingMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_transaction_execution_us"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},status={:?},context={:?},call_message={},sequencer={} value={},rollup_height={}",
            self.measurement_name(),
            // tags
            self.tx_effect,
            self.execution_context,
            self.call_message,
            self.sequencer_address,
            //fields
            self.execution_time.as_micros(),
            self.visible_slot_number,
        )?;
        if self.gas_used.len() >= 2 {
            write!(
                buffer,
                ",gas_used_compute={},gas_used_mem={}",
                self.gas_used[0], self.gas_used[1],
            )?;
        }

        Ok(())
    }
}

impl Metric for SlotProcessingMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_slot_execution_time_us"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},context={:?} blobs_selection={},finalization={},visible_slot_number={},da_height={}",
            self.measurement_name(),
            // Tags
            self.execution_context,
            // Fields
            self.blobs_selection_time.as_micros(),
            self.slot_finalization_time.as_micros(),
            self.visible_slot_number,
            self.da_height,
        )?;
        if self.gas_used.len() >= 2 {
            write!(
                buffer,
                ",gas_used_compute={},gas_used_mem={}",
                self.gas_used[0], self.gas_used[1],
            )?;
        }
        Ok(())
    }
}

impl Metric for UserSpaceSlotProcessingMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_slot_execution_time_us"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},context={:?} begin_hooks={},blobs_processing={},end_hooks={},rollup_height={}",
            self.measurement_name(),
            // Tags
            self.execution_context,
            // Fields
            self.begin_block_hook_time.as_micros(),
            self.blobs_processing_time.as_micros(),
            self.end_block_hook_time.as_micros(),
            self.visible_slot_number,
        )?;
        if self.gas_used.len() >= 2 {
            write!(
                buffer,
                ",gas_used_compute={},gas_used_mem={}",
                self.gas_used[0], self.gas_used[1],
            )?;
        }
        Ok(())
    }
}

/// Simplified version of a batch outcome.
#[derive(Debug)]
pub enum BatchOutcome {
    #[allow(missing_docs)]
    Executed,
    #[allow(missing_docs)]
    Ignored,
}

/// Metrics for batch with transactions.
#[derive(Debug)]
pub struct BatchMetrics {
    #[allow(missing_docs)]
    pub processing_time: std::time::Duration,
    /// Number of transactions have been processed in batch.
    pub transactions_count: usize,
    /// Number of transactions have been ignored..
    pub ignored_transactions_count: usize,
}

impl Metric for BatchMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_batch_processing"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{} processing_time_us={},transactions={},ignored_transactions={}",
            self.measurement_name(),
            self.processing_time.as_micros(),
            self.transactions_count,
            self.ignored_transactions_count,
        )
    }
}

/// Metrics for an HTTP subsystem.
/// Can be applied to REST API or JSON RPC.
#[derive(Debug)]
pub struct HttpMetrics {
    /// HTTP method.
    pub request_method: http::Method,
    /// URI being requested.
    pub request_uri: http::Uri,
    /// Status code of the response.
    pub response_status: http::StatusCode,
    /// Approximate size of the response body.
    pub response_body_size: u64,
    /// Time it took for the inner handler to finish processing.
    /// Does not include request reading and response writing.
    pub handler_processing_time: std::time::Duration,
}

impl Metric for HttpMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_http_handlers"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},req_method={},resp_status={},path={} processing_time_us={},response_body_bytes={}",
            self.measurement_name(),
            // Tags
            self.request_method,
            self.response_status.as_u16(),
            self.request_uri.path(),
            // Fields
            self.handler_processing_time.as_micros(),
            self.response_body_size,
        )
    }
}

/// Representation of cycle count and free heap for a particular chunk of execution inside ZK VM guest.
#[derive(Debug)]
pub struct ZkVmExecutionChunk {
    /// Name of the caller site, usually a function or method
    pub name: String,
    /// Metadata associated with the metric. Usually input values collected from the caller function
    pub metadata: Vec<(String, String)>,
    /// A number of ZKVM cycles have been spent on this call.
    pub cycles_count: u64,
    /// Available bytes on the heap after execution of the block is complete.
    pub free_heap_bytes: u64,
    /// Amount of bytes of memory used during the execution of the block (and not reclaimed after execution is complete).
    pub memory_used: u64,
}

impl Metric for ZkVmExecutionChunk {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_zkvm"
    }

    #[cfg(feature = "gas-constant-estimation")]
    fn write_to_csv(&self, writers: &mut super::csv_helper::CsvWriters) -> std::io::Result<()> {
        let writer = &mut writers.zk_vm_writer;

        let meta = &self.metadata;
        let maybe_pre_state_root = meta.iter().find(|(k, _)| k == "pre_state_root");
        if let Some(pre_state_root) = maybe_pre_state_root {
            let row = format!(
                "{},{},{},{},{}\n",
                self.name,
                self.cycles_count,
                self.memory_used,
                self.free_heap_bytes,
                pre_state_root.1
            );
            writer.write_all(row.as_bytes())?;
            writer.flush()?;
        }
        Ok(())
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        // We are adding the metadata as measurmement tags in the influxdb line protocol.
        let metadata = self
            .metadata
            .iter()
            .map(|(key, value)| {
                // Uses special telegraf formatting
                let telegraf_formatted_key = safe_telegraf_string(key);

                format!("{telegraf_formatted_key}={value}")
            })
            .collect::<Vec<_>>()
            .join(",");

        write!(
            buffer,
            "{},name={}{metadata} cycles_count={},free_heap_bytes={},memory_used={}",
            self.measurement_name(),
            self.name,
            self.cycles_count,
            self.free_heap_bytes,
            self.memory_used
        )
    }
}

#[derive(Debug)]
#[allow(missing_docs)]
pub enum ZkCircuit {
    Inner,
    Outer,
}

/// How much wall clock time it took to run prover
#[derive(Debug)]
pub struct ZkProvingTime {
    #[allow(missing_docs)]
    pub proving_time: std::time::Duration,
    #[allow(missing_docs)]
    pub is_success: bool,
    #[allow(missing_docs)]
    pub zk_circuit: ZkCircuit,
}

impl Metric for ZkProvingTime {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_zkvm_proving"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(
            buffer,
            "{},is_success={},circuit={:?} proving_time_ms={}",
            self.measurement_name(),
            self.is_success,
            self.zk_circuit,
            self.proving_time.as_millis()
        )
    }
}

/// Metrics on recent state accesses
#[derive(Debug)]
pub struct StateMetrics {
    #[allow(missing_docs)]
    accesses: Vec<StateAccessMetric>,
}

impl Default for StateMetrics {
    fn default() -> Self {
        Self {
            accesses: Vec::with_capacity(Self::MAX_CAPACITY),
        }
    }
}

impl StateMetrics {
    #[cfg(feature = "native")]
    const MAX_CAPACITY: usize = 100;

    #[cfg(not(feature = "native"))]
    const MAX_CAPACITY: usize = 0;

    /// Pushes a new state access metric.
    pub fn push(&mut self, metric: StateAccessMetric) {
        if self.accesses.len() < Self::MAX_CAPACITY {
            self.accesses.push(metric);
        }
    }

    /// Takes the state access metrics.
    pub fn take(&mut self) -> Vec<StateAccessMetric> {
        std::mem::take(&mut self.accesses)
    }

    /// Returns the number of accesses. in the current state metrics.
    pub fn len(&self) -> usize {
        self.accesses.len()
    }

    /// Returns true if there are no state accesses since the last flush
    pub fn is_empty(&self) -> bool {
        self.accesses.is_empty()
    }
}

/// Metrics for a single state access.
#[derive(Debug)]
pub struct StateAccessMetric {
    #[allow(missing_docs)]
    pub op: &'static str,
    #[allow(missing_docs)]
    pub key_size: usize,
    #[allow(missing_docs)]
    pub storage_read_size: Option<u32>,
    #[allow(missing_docs)]
    pub duration: MaybeTimer,
}

impl StateAccessMetric {
    /// Creates a new state access metric.
    pub fn new(op: &'static str, key_size: usize) -> Self {
        Self {
            op,
            key_size,
            storage_read_size: None,
            duration: MaybeTimer::started(),
        }
    }
}

fn summarize(
    metrics: &[StateAccessMetric],
    prefix: &str,
    target: &mut Vec<u8>,
) -> std::io::Result<()> {
    let total_reads = metrics.len();
    let mut cache_misses = 0;
    let mut cache_miss_bytes = 0;
    let mut slowest_read = std::time::Duration::from_millis(0);
    let mut slowest_read_name = "none";
    let mut slowest_read_key_size = 0;
    let mut slowest_read_storage_read_size = 0;
    for metric in metrics {
        cache_misses += metric.storage_read_size.is_some() as usize;
        cache_miss_bytes += metric.storage_read_size.unwrap_or(0);
        if metric.duration.elapsed() > slowest_read {
            slowest_read = metric.duration.elapsed();
            slowest_read_name = metric.op;
            slowest_read_key_size = metric.key_size;
            slowest_read_storage_read_size = metric.storage_read_size.unwrap_or(0);
        }
    }
    write!(
        target,
        "{prefix}_total_reads={total_reads},{prefix}_cache_misses={cache_misses},{prefix}_cache_miss_bytes={cache_miss_bytes}",
    )?;
    write!(target, "{prefix}_slowest_read={},{prefix}_slowest_read_name={slowest_read_name},{prefix}_slowest_read_key_size={slowest_read_key_size},{prefix}_slowest_read_storage_read_size={slowest_read_storage_read_size}", slowest_read.as_micros())?;
    Ok(())
}

// fn serialize_state_access_metric(buffer: &mut Vec<u8>, metric: &StateAccessMetric, prefix: &str) -> std::io::Result<()> {
//     write!(buffer, "{}_op={},{}_key_size={},{}_storage_read_size={},{}_duration_us={}", prefix, metric.op, prefix, metric.key_size, prefix, metric.storage_read_size.unwrap_or(0), prefix, metric.duration.elapsed().as_micros())
// }

/// Metrics for `auth_and_process_tx`, and the tx hash
#[derive(Debug)]
pub struct AuthAndProcessMetrics {
    /// The transaction hash
    pub tx_hash: HexHash,
    /// The metrics.
    pub timings: AuthAndProcessTimings,
}

impl AuthAndProcessMetrics {
    /// Creates a new `AuthAndProcessMetrics` instance.
    pub fn new(tx_hash: HexHash, timings: AuthAndProcessTimings) -> Self {
        Self { tx_hash, timings }
    }
}

impl Metric for AuthAndProcessMetrics {
    fn measurement_name(&self) -> &'static str {
        "sov_rollup_auth_and_process_metrics"
    }

    // TODO: Finish serializing all the metrics
    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        write!(buffer, "{}, tx_hash={},total_time_us={},auth_time_us={},resolve_context_time_us={},check_uniqueness_time_us={},mark_tx_attempted_time_us={},attempt_tx_time_us={},reserve_gas_time_us={},refund_remaining_gas_time_us={},reward_prover_time_us={}", self.measurement_name(), self.tx_hash, self.timings.auth.elapsed().as_micros(), self.timings.total_timer.elapsed().as_micros(), self.timings.resolve_context_timer.elapsed().as_micros(), self.timings.check_uniqueness_timer.elapsed().as_micros(), self.timings.mark_tx_attempted_timer.elapsed().as_micros(), self.timings.attempt_tx_timer.elapsed().as_micros(), self.timings.reserve_gas_timer.elapsed().as_micros(), self.timings.refund_remaining_gas_timer.elapsed().as_micros(), self.timings.reward_prover_timer.elapsed().as_micros())?;
        summarize(
            &self.timings.resolve_context_access_metrics,
            "resolve_context",
            buffer,
        )?;
        summarize(
            &self.timings.check_uniqueness_access_metrics,
            "check_uniqueness",
            buffer,
        )?;
        summarize(
            &self.timings.mark_tx_attempted_access_metrics,
            "mark_tx_attempted",
            buffer,
        )?;
        summarize(
            &self.timings.attempt_tx_access_metrics,
            "attempt_tx",
            buffer,
        )?;
        summarize(
            &self.timings.reserve_gas_access_metrics,
            "reserve_gas",
            buffer,
        )?;
        summarize(
            &self.timings.refund_remaining_gas_access_metrics,
            "refund_remaining_gas",
            buffer,
        )?;
        summarize(
            &self.timings.reward_prover_access_metrics,
            "reward_prover",
            buffer,
        )?;
        Ok(())
    }
}

/// Timings for `auth_and_process_tx`
#[derive(Debug, Default)]
pub struct AuthAndProcessTimings {
    /// Time to deserialize and authenticate the tx. Includes no state access in the starter
    pub auth: MaybeTimer,
    /// The total time it took to authenticate and process the tx.
    pub total_timer: MaybeTimer,
    /// Timer for resolving the context.
    pub resolve_context_timer: MaybeTimer,
    /// State Accesses performed while resolving the context.
    pub resolve_context_access_metrics: Vec<StateAccessMetric>,
    /// Timer for checking uniqueness.
    pub check_uniqueness_timer: MaybeTimer,
    /// State Accesses performed while checking uniqueness.
    pub check_uniqueness_access_metrics: Vec<StateAccessMetric>,
    /// Timer for marking the tx as attempted.
    pub mark_tx_attempted_timer: MaybeTimer,
    /// State Accesses performed while marking the tx as attempted.
    pub mark_tx_attempted_access_metrics: Vec<StateAccessMetric>,
    /// Timer for executing the tx.
    pub attempt_tx_timer: MaybeTimer,
    /// State Accesses performed while executing the tx.
    pub attempt_tx_access_metrics: Vec<StateAccessMetric>,
    /// Timer for reserving gas.
    pub reserve_gas_timer: MaybeTimer,
    /// State Accesses performed while reserving gas.
    pub reserve_gas_access_metrics: Vec<StateAccessMetric>,
    /// Timer for refunding remaining gas.
    pub refund_remaining_gas_timer: MaybeTimer,
    /// State Accesses performed while refunding remaining gas.
    pub refund_remaining_gas_access_metrics: Vec<StateAccessMetric>,
    /// Timer for rewarding the prover.
    pub reward_prover_timer: MaybeTimer,
    /// State Accesses performed while rewarding the prover.
    pub reward_prover_access_metrics: Vec<StateAccessMetric>,
}
