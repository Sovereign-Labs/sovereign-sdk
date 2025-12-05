use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use sov_bank::Bank;
use sov_bank::CallMessageDiscriminants::Transfer;
use sov_mock_da::BlockProducingConfig;
use sov_modules_api::capabilities::config_chain_id;
use sov_modules_api::prelude::arbitrary::{self, Unstructured};
use sov_modules_api::prelude::tracing;
use sov_modules_api::transaction::TxDetails;
use sov_modules_api::{CryptoSpec, DispatchCall, EncodeCall, PrivateKey, Runtime, Spec};
use sov_synthetic_load::CallMessageDiscriminants::{
    ReadAndSetHeavyState, ReadAndSetManyIndividualValues, RunCPUHeavyOperation,
};
use sov_synthetic_load::SyntheticLoad;
use sov_test_state_consistency::StateConsistency;
use sov_test_utils::{TransactionType, TEST_DEFAULT_MAX_FEE, TEST_DEFAULT_MAX_PRIORITY_FEE};
use sov_transaction_generator::generators::bank::harness_interface::BankHarness;
use sov_transaction_generator::generators::bank::BankMessageGenerator;
use sov_transaction_generator::generators::basic::{
    BasicCallMessageFactory, BasicChangeLogEntry, BasicModuleRef, BasicTag,
};
use sov_transaction_generator::generators::state_consistency::{
    StateConsistencyHarness, StateConsistencyMessageGenerator,
};
use sov_transaction_generator::generators::synthetic_load::{
    SyntheticLoadHarness, SyntheticLoadMessageGenerator,
};
use sov_transaction_generator::interface::rng_utils::{get_random_bytes, randomize_buffer};
use sov_transaction_generator::interface::MessageValidity;
use sov_transaction_generator::{Distribution, GeneratedMessage, Percent, State};
use tokio::sync::mpsc;
use tokio::sync::watch::Receiver;

pub const DEFAULT_BLOCK_TIME_MS: u64 = 200;
pub const DEFAULT_BLOCK_PRODUCING_CONFIG: BlockProducingConfig = BlockProducingConfig::Periodic {
    block_time_ms: DEFAULT_BLOCK_TIME_MS,
};

pub const DEFAULT_FINALIZATION_BLOCKS: u32 = 5;

pub const BUFFER_SIZE: usize = 100_000;
// The minimum randomness needed to guarantee successful transaction generation
pub const SAFE_MIN_RANDOMNESS: usize = 1_000;

/// Statistics message sent from worker to watcher
#[derive(Debug, Clone)]
pub struct WorkerStats {
    pub txn_count: usize,
    pub elapsed_secs: f64,
}

pub fn plain_tx_with_default_details<R: Runtime<S>, S: Spec>(
    gen_output: &GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>>,
) -> TransactionType<R, S> {
    TransactionType::Plain {
        message: gen_output.message.clone(),
        key: gen_output.sender.clone(),
        details: TxDetails {
            max_priority_fee_bips: TEST_DEFAULT_MAX_PRIORITY_FEE,
            max_fee: TEST_DEFAULT_MAX_FEE,
            gas_limit: None,
            chain_id: config_chain_id(),
        },
    }
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum ValidityProfile {
    /// Only valid transactions
    Clean,
    /// 5% of invalid transactions
    Buzzy,
    /// 50/50
    Half,
    /// 90% of invalid transactions
    Spammy,
}

impl ValidityProfile {
    pub fn get_validity(&self) -> Distribution<MessageValidity> {
        match self {
            ValidityProfile::Clean => Distribution::with_values(vec![(1, MessageValidity::Valid)]),
            ValidityProfile::Buzzy => Distribution::with_values(vec![
                (20, MessageValidity::Valid),
                (1, MessageValidity::Invalid),
            ]),
            ValidityProfile::Half => Distribution::with_values(vec![
                (50, MessageValidity::Valid),
                (50, MessageValidity::Invalid),
            ]),
            ValidityProfile::Spammy => Distribution::with_values(vec![
                (10, MessageValidity::Valid),
                (90, MessageValidity::Invalid),
            ]),
        }
    }
}

/// A runner for the soak tests, providing helpers for setting up modules to be used in the soak
/// test.
///
/// Modules can be seleted dynamically but only if the Runtime includes them.
///
/// # Example
/// ```ignore
/// SoakTestBuilder::new()
///     .with_bank()
///     .with_state_consistency()
///     .run(client, rx, worker_id, num_workers, validity).await?;
/// ```
#[derive(Default)]
pub struct SoakTestRunner<R, S: Spec> {
    modules: Vec<BasicModuleRef<S, R>>,
}

impl<R, S> SoakTestRunner<R, S>
where
    R: Runtime<S> + Clone,
    S: Spec,
{
    /// Create a new builder with no modules.
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    /// Add Bank module to the soak test.
    ///
    /// This method is only available if your Runtime implements `EncodeCall<Bank<S>>`.
    pub fn with_bank(mut self) -> Self
    where
        R: EncodeCall<Bank<S>>,
    {
        let bank_harness = BankHarness::new(BankMessageGenerator::<S>::new(
            Distribution::with_equiprobable_values(vec![Transfer]),
            Percent::fifty(),
        ));
        self.modules.push(Arc::new(bank_harness));
        self
    }

    /// Add SyntheticLoad module to the soak test.
    ///
    /// This method is only available if your Runtime implements `EncodeCall<SyntheticLoad<S>>`.
    pub fn with_synthetic_load(mut self) -> Self
    where
        R: EncodeCall<SyntheticLoad<S>>,
    {
        let synthetic_load_admin = <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey::generate();
        let synthetic_load_harness = SyntheticLoadHarness::new(SyntheticLoadMessageGenerator::new(
            Distribution::with_equiprobable_values(vec![
                ReadAndSetManyIndividualValues,
                ReadAndSetHeavyState,
                RunCPUHeavyOperation,
            ]),
            sov_transaction_generator::generators::synthetic_load::SyntheticLoadGeneratorOptions {
                maximum_vec_length: 10,
                min_and_max_number_of_individual_state_operations: (1, 10000),
                min_and_max_number_of_new_values_for_heavy_state: (100, 1000),
                min_and_max_number_of_iterations_for_cpu_heavy_operation: (1000, 5000),
                max_heavy_state_size: 1_000_000,
            },
            synthetic_load_admin,
        ));
        self.modules.push(Arc::new(synthetic_load_harness));
        self
    }

    /// Add StateConsistency module to the soak test.
    ///
    /// This method is only available if your Runtime implements `EncodeCall<StateConsistency<S>>`.
    pub fn with_state_consistency(mut self) -> Self
    where
        R: EncodeCall<StateConsistency<S>>,
    {
        let state_consistency_harness =
            StateConsistencyHarness::new(StateConsistencyMessageGenerator::default());
        self.modules.push(Arc::new(state_consistency_harness));
        self
    }

    /// Run the soak test with the configured modules.
    ///
    /// # Parameters
    /// - `client`: The API client for submitting transactions
    /// - `rx`: Receiver to signal when to stop the test
    /// - `worker_id`: Unique identifier for this worker
    /// - `num_workers`: Total number of parallel workers
    /// - `validity`: Distribution of valid vs invalid messages
    /// - `restart_after`: Optional duration after which to restart the worker
    /// - `stats_tx`: Channel sender for reporting throughput statistics
    pub async fn run(
        self,
        client: sov_api_spec::Client,
        rx: Receiver<bool>,
        mut worker_id: u128,
        num_workers: u32,
        validity: Distribution<MessageValidity>,
        restart_after: Option<std::time::Duration>,
        stats_tx: mpsc::UnboundedSender<WorkerStats>,
    ) -> anyhow::Result<()> {
        loop {
            tracing::info!(worker_id, ?restart_after, "Starting worker");
            let result = if let Some(duration) = restart_after {
                tokio::select! {
                    result = prepare_and_send_txs(
                        self.modules.clone(),
                        &client,
                        rx.clone(),
                        worker_id,
                        num_workers,
                        validity.clone(),
                        stats_tx.clone(),
                    ) => {
                        // If prepare_and_send_txs completes (likely an error), return immediately
                        return result;
                    }
                    _ = tokio::time::sleep(duration) => {
                        // Timer expired, restart with incremented worker_id
                        tracing::info!("Timer expired for worker {worker_id}, restarting with new worker_id");
                        worker_id += num_workers as u128;
                        continue;
                    }
                }
            } else {
                // No restart timer, just run until completion
                prepare_and_send_txs(
                    self.modules.clone(),
                    &client,
                    rx.clone(),
                    worker_id,
                    num_workers,
                    validity.clone(),
                    stats_tx.clone(),
                )
                .await
            };

            return result;
        }
    }
}

pub struct TestGenerator<R: Runtime<S>, S: Spec> {
    generator: BasicCallMessageFactory<S, R>,
    state: State<S, BasicTag>,
    randomness: Vec<u8>,
    remaining_randomness: usize,
    target_buffer_size: usize,
    salt: u128,
}

impl<R: Runtime<S>, S: Spec> TestGenerator<R, S> {
    pub fn generate(
        &mut self,
        modules_distribution: &Distribution<BasicModuleRef<S, R>>,
        validity: MessageValidity,
    ) -> GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>> {
        for _ in 0..20 {
            if self.has_enough_randomness() {
                let u =
                    &mut arbitrary::Unstructured::new(&self.randomness[self.randomness_offset()..]);

                if let Ok(output) = self.generator.generate_call_message(
                    modules_distribution,
                    u,
                    &mut self.state,
                    validity,
                ) {
                    self.remaining_randomness = u.len();
                    return output;
                } else {
                    self.target_buffer_size *= 2;
                }
            }
            self.re_randomize();
        }
        unreachable!("Could not get enough randomness to generate a transaction");
    }

    fn re_randomize(&mut self) {
        if self.randomness.len() < self.target_buffer_size {
            self.randomness = vec![0; self.target_buffer_size];
        }
        randomize_buffer(&mut self.randomness[..], self.salt);
        self.remaining_randomness = self.randomness.len();
        self.salt += 1;
    }

    fn randomness_offset(&self) -> usize {
        self.randomness.len() - self.remaining_randomness
    }

    fn has_enough_randomness(&self) -> bool {
        self.remaining_randomness > std::cmp::min(SAFE_MIN_RANDOMNESS, self.target_buffer_size / 10)
    }
}

// Setup generation with the given params
pub fn setup_harness<R: Runtime<S> + Clone, S: Spec>(rng_salt: u128) -> TestGenerator<R, S> {
    let factory = BasicCallMessageFactory::<S, R>::new();
    let state: State<S, BasicTag> = State::new();

    let random_bytes: Vec<u8> = get_random_bytes(100_000, rng_salt);
    let u = &mut arbitrary::Unstructured::new(&random_bytes[..]);
    let remaining_randomness = u.len();
    TestGenerator {
        randomness: random_bytes,
        remaining_randomness,
        generator: factory,
        state,
        target_buffer_size: BUFFER_SIZE,
        salt: rng_salt,
    }
}

/// Per-worker statistics tracking
#[derive(Debug, Default)]
struct WorkerStatTracker {
    /// Total transactions sent
    total_txns: usize,
    /// Total time spent sending transactions
    total_time: f64,
    /// Throughput samples for computing variability
    throughput_samples: Vec<f64>,
}

impl WorkerStatTracker {
    fn update(&mut self, stats: &WorkerStats) {
        self.total_txns += stats.txn_count;
        self.total_time += stats.elapsed_secs;

        // Calculate instantaneous throughput for this batch
        if stats.elapsed_secs > 0.0 {
            let throughput = stats.txn_count as f64 / stats.elapsed_secs;
            self.throughput_samples.push(throughput);
        }
    }

    fn average_throughput(&self) -> f64 {
        if self.total_time > 0.0 {
            self.total_txns as f64 / self.total_time
        } else {
            0.0
        }
    }

    /// Calculate coefficient of variation (CV) for throughput
    /// CV = standard_deviation / mean
    fn throughput_variability(&self) -> f64 {
        if self.throughput_samples.is_empty() {
            return 0.0;
        }

        let mean: f64 =
            self.throughput_samples.iter().sum::<f64>() / self.throughput_samples.len() as f64;

        if mean == 0.0 {
            return 0.0;
        }

        let variance: f64 = self
            .throughput_samples
            .iter()
            .map(|&x| (x - mean).powi(2))
            .sum::<f64>()
            / self.throughput_samples.len() as f64;

        let std_dev = variance.sqrt();
        std_dev / mean // Coefficient of variation
    }
}

/// Spawn a watcher task that monitors and reports statistics from all workers
///
/// # Parameters
/// - `receivers`: Vector of mpsc receivers, one per worker
/// - `report_interval_secs`: How often to print statistics (in seconds)
///
/// # Returns
/// A future that runs until all workers have closed their senders
pub async fn spawn_throughput_watcher(
    mut receivers: Vec<mpsc::UnboundedReceiver<WorkerStats>>,
    report_interval_secs: u64,
) {
    let mut worker_stats: HashMap<usize, WorkerStatTracker> = HashMap::new();
    let mut report_interval = tokio::time::interval(Duration::from_secs(report_interval_secs));
    let start_time = std::time::Instant::now();

    // Skip the first tick which fires immediately
    report_interval.tick().await;

    loop {
        tokio::select! {
            // Poll all receivers for new stats
            _ = async {
                for (worker_id, receiver) in receivers.iter_mut().enumerate() {
                    while let Ok(stats) = receiver.try_recv() {
                        worker_stats.entry(worker_id)
                            .or_insert_with(WorkerStatTracker::default)
                            .update(&stats);
                    }
                }
                // Small delay to avoid busy-waiting
                tokio::time::sleep(Duration::from_millis(100)).await;
            } => {},

            // Periodic reporting - this will fire every report_interval_secs
            _ = report_interval.tick() => {
                if worker_stats.is_empty() {
                    continue;
                }

                // Calculate true system throughput using wall-clock time
                let wall_clock_secs = start_time.elapsed().as_secs_f64();
                let total_txns: usize = worker_stats.values().map(|w| w.total_txns).sum();
                let total_throughput = total_txns as f64 / wall_clock_secs;

                // Per-worker throughputs (based on active sending time) for variance analysis
                let worker_throughputs: Vec<f64> = worker_stats
                    .values()
                    .map(|w| w.average_throughput())
                    .collect();
                let avg_worker_throughput = worker_throughputs.iter().sum::<f64>() / worker_throughputs.len() as f64;

                // Variance in throughput between workers
                let throughput_stddev = if worker_throughputs.len() > 1 {
                    let variance: f64 = worker_throughputs
                        .iter()
                        .map(|&t| (t - avg_worker_throughput).powi(2))
                        .sum::<f64>() / worker_throughputs.len() as f64;
                    variance.sqrt()
                } else {
                    0.0
                };

                // Average variability per worker
                let worker_variabilities: Vec<f64> = worker_stats
                    .values()
                    .map(|w| w.throughput_variability())
                    .collect();
                let avg_worker_variability = worker_variabilities.iter().sum::<f64>()
                    / worker_variabilities.len() as f64;

                // Calculate average worker utilization (active sending time / wall clock time)
                let total_active_time: f64 = worker_stats.values().map(|w| w.total_time).sum();
                let avg_worker_utilization = total_active_time / (wall_clock_secs * worker_stats.len() as f64);

                tracing::info!(
                    "\n========== THROUGHPUT STATISTICS ==========\n\
                     Total Transactions:      {}\n\
                     Wall Clock Time:         {:.1}s\n\
                     System Throughput:       {:.2} tx/s\n\
                     -------------------------------------------\n\
                     Avg Worker Send Rate:    {:.2} tx/s (active time)\n\
                     Avg Worker Utilization:  {:.1}%\n\
                     Inter-Worker StdDev:     {:.2} tx/s ({:.1}% of avg)\n\
                     Avg Intra-Worker CV:     {:.2} ({:.1}% variability)\n\
                     Active Workers:          {}\n\
                     ===========================================",
                    total_txns,
                    wall_clock_secs,
                    total_throughput,
                    avg_worker_throughput,
                    avg_worker_utilization * 100.0,
                    throughput_stddev,
                    if avg_worker_throughput > 0.0 { (throughput_stddev / avg_worker_throughput) * 100.0 } else { 0.0 },
                    avg_worker_variability,
                    avg_worker_variability * 100.0,
                    worker_stats.len(),
                );
            }
        }

        // Check if all receivers are closed
        if receivers.iter().all(|r| r.is_closed()) {
            tracing::info!("All workers have finished. Watcher exiting.");
            break;
        }
    }
}

async fn prepare_and_send_txs<R: Runtime<S> + Clone, S: Spec>(
    modules: Vec<BasicModuleRef<S, R>>,
    client: &sov_api_spec::Client,
    rx: Receiver<bool>,
    worker_id: u128,
    num_workers: u32,
    validity: Distribution<MessageValidity>,
    stats_tx: mpsc::UnboundedSender<WorkerStats>,
) -> anyhow::Result<()> {
    let mut nonces: HashMap<<<S as Spec>::CryptoSpec as CryptoSpec>::PublicKey, u64> =
        Default::default();

    let modules = Distribution::with_equiprobable_values(modules);
    let random_bytes = get_random_bytes(100_000_000, worker_id);
    let u = &mut Unstructured::new(&random_bytes[..]);

    let mut generator: TestGenerator<R, S> = setup_harness::<R, _>(worker_id);
    let worker_start = std::time::Instant::now();
    let mut total_txns = 0;

    while !*rx.borrow() {
        // Generate both values while RNG is in scope, then await after it drops.
        let (txn_count, sleep_ms) = {
            // rng must fall out of scope before awaiting anything so this fn is Send
            let mut rng = rand::thread_rng();

            // Do this at the start so we add some jitter to initial API requests
            let sleep_ms = rng.gen_range(25..100);
            let txn_count = rng.gen_range(10..100);
            (txn_count, sleep_ms)
        };

        // Use cooperative sleep to avoid blocking the async runtime
        tokio::time::sleep(Duration::from_millis(sleep_ms)).await;

        let mut txns = vec![];
        for _ in 0..txn_count {
            let validity = validity.select_value(u)?;
            let msg = generator.generate(&modules, *validity);
            let tx = plain_tx_with_default_details::<R, S>(&msg);
            let signed_tx = {
                let TransactionType::Plain {
                    message,
                    key,
                    details,
                } = tx
                else {
                    panic!("The method `plain_tx_with_default_details` should return a plain transaction!");
                };

                let pub_key = key.clone().pub_key();
                let nonce = nonces.get(&pub_key).unwrap_or(&0);

                // If message is invalid, create a future nonce and send it to the sequencer
                // The sequencer should reject the transaction as invalid, and more importantly, the sequencer should not update the nonce for the given account.
                if *validity == MessageValidity::Invalid {
                    let mut future_nonce = HashMap::from([(pub_key, nonce + 1000)]);
                    let invalid_tx = TransactionType::<R, S>::sign(
                        message.clone(),
                        key.clone(),
                        &R::CHAIN_HASH,
                        details.clone(),
                        &mut future_nonce,
                    );
                    txns.push((invalid_tx, true));
                    continue;
                }
                TransactionType::<R, S>::sign(message, key, &R::CHAIN_HASH, details, &mut nonces)
            };
            txns.push((signed_tx, false));
        }

        let start = std::time::Instant::now();
        for (tx, is_invalid) in &txns {
            if *is_invalid {
                client
                    .send_tx_to_sequencer(tx)
                    .await
                    .expect_err("Outdated transaction should have failed");
            } else {
                client.send_tx_to_sequencer_with_retry(tx).await?;
            }
            total_txns += 1;
        }
        let elapsed = start.elapsed();
        tracing::debug!(id = %worker_id, "Sent {} transactions in {}ms. Current throughput: {:.2} txs per second. Running throughput: {:.2} txs per second", txns.len(), elapsed.as_millis(), (txns.len() * num_workers as usize) as f64 / elapsed.as_secs_f64(), (total_txns * num_workers as usize) as f64 / worker_start.elapsed().as_secs_f64());

        // Send stats to watcher
        stats_tx.send(WorkerStats {
            txn_count: txns.len(),
            elapsed_secs: elapsed.as_secs_f64(),
        }).expect("Worker failed to send stats update to aggregator task - aggregated throughput will not be accurate! Panicking.");
    }

    Ok(())
}
