use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng;
use sov_bank::Bank;
use sov_bank::CallMessageDiscriminants::Transfer;
use sov_modules_api::capabilities::{config_chain_id, UniquenessData};
use sov_modules_api::prelude::arbitrary::{self, Arbitrary, Unstructured};
use sov_modules_api::prelude::tracing;
use sov_modules_api::transaction::{Transaction, TxDetails, UnsignedTransaction};
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
use sov_transaction_generator::generators::value_setter::ValueSetterChangeLogEntry;
use sov_transaction_generator::interface::rng_utils::{get_random_bytes, randomize_buffer};
use sov_transaction_generator::interface::MessageValidity;
use sov_transaction_generator::{
    Distribution, GeneratedMessage, HarnessModule, MessageOutcome, Percent, State,
};
use sov_value_setter::ValueSetter;
use tokio::sync::watch::Receiver;

pub const BUFFER_SIZE: usize = 100_000;
// The minimum randomness needed to guarantee successful transaction generation
pub const SAFE_MIN_RANDOMNESS: usize = 1_000;
const TX_SEND_RETRY_ATTEMPTS: u32 = 5000;
const TX_SEND_RETRY_DELAY: Duration = Duration::from_millis(50);
const TX_ATTEMPT_STATS_INTERVAL: Duration = Duration::from_secs(1);

static TX_ACCEPTED_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static TX_REJECTED_ATTEMPTS: AtomicU64 = AtomicU64::new(0);
static TX_ATTEMPT_STATS_LOGGER: OnceLock<()> = OnceLock::new();

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

fn current_timestamp_micros() -> anyhow::Result<u64> {
    let micros = SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros();
    Ok(u64::try_from(micros)?)
}

fn sign_with_current_generation<R: Runtime<S>, S: Spec>(
    message: <R as DispatchCall>::Decodable,
    key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    details: TxDetails<S>,
) -> anyhow::Result<Transaction<R, S>> {
    Ok(Transaction::<R, S>::new_signed_tx(
        &key,
        &R::CHAIN_HASH,
        UnsignedTransaction::new(
            message,
            details.chain_id,
            details.max_priority_fee_bips,
            details.max_fee,
            UniquenessData::Generation(current_timestamp_micros()?),
            details.gas_limit,
        ),
    ))
}

fn record_tx_attempt(accepted: bool) {
    if accepted {
        TX_ACCEPTED_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    } else {
        TX_REJECTED_ATTEMPTS.fetch_add(1, Ordering::Relaxed);
    }
}

fn start_tx_attempt_stats_logger() {
    TX_ATTEMPT_STATS_LOGGER.get_or_init(|| {
        tokio::spawn(async {
            loop {
                tokio::time::sleep(TX_ATTEMPT_STATS_INTERVAL).await;

                let accepted_attempts = TX_ACCEPTED_ATTEMPTS.swap(0, Ordering::Relaxed);
                let rejected_attempts = TX_REJECTED_ATTEMPTS.swap(0, Ordering::Relaxed);
                let total_attempts = accepted_attempts + rejected_attempts;
                let rejection_rate = if total_attempts == 0 {
                    0.0
                } else {
                    rejected_attempts as f64 * 100.0 / total_attempts as f64
                };

                tracing::info!(
                    accepted_attempts,
                    rejected_attempts,
                    rejection_rate_pct = rejection_rate,
                    "Soak transaction attempt stats"
                );
            }
        });
    });
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
    use_retries: bool,
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
            use_retries: true,
        }
    }

    /// By default, all transactions that are intended to succeed are submitted with retries using a
    /// fixed 50ms delay.
    /// When opted out, transactions are sent using a single request and fail-fast if they're not
    /// successful on the first try.
    pub fn without_retries(mut self) -> Self {
        self.use_retries = false;
        self
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

    /// Add a `ValueSetter` module harness that only emits `SetValueAndSleep` calls.
    ///
    /// This method is only available if your Runtime implements `EncodeCall<ValueSetter<S>>`.
    pub fn with_value_setter_sleep(
        mut self,
        admin_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
        sleep_millis: u64,
    ) -> Self
    where
        R: EncodeCall<ValueSetter<S>>,
    {
        self.modules.push(Arc::new(ValueSetterSleepHarness::new(
            admin_key,
            sleep_millis,
        )));
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
    pub async fn run(
        self,
        client: sov_api_spec::Client,
        rx: Receiver<bool>,
        mut worker_id: u128,
        num_workers: u32,
        validity: Distribution<MessageValidity>,
        restart_after: Option<std::time::Duration>,
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
                        self.use_retries
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
                    self.use_retries,
                )
                .await
            };

            return result;
        }
    }
}

#[derive(Debug, Clone)]
struct ValueSetterSleepHarness<S: Spec> {
    admin_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    sleep_millis: u64,
}

impl<S: Spec> ValueSetterSleepHarness<S> {
    fn new(
        admin_key: <<S as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
        sleep_millis: u64,
    ) -> Self {
        Self {
            admin_key,
            sleep_millis,
        }
    }
}

impl<R, S> HarnessModule<S, R, BasicTag, BasicChangeLogEntry<S>> for ValueSetterSleepHarness<S>
where
    R: EncodeCall<ValueSetter<S>>,
    S: Spec,
{
    fn generate_setup_messages(
        &self,
        _u: &mut Unstructured<'_>,
        _generator_state: &mut State<S, BasicTag>,
    ) -> arbitrary::Result<
        Vec<GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>>>,
    > {
        Ok(Vec::new())
    }

    fn generate_call_message(
        &self,
        u: &mut Unstructured<'_>,
        _generator_state: &mut State<S, BasicTag>,
        _validity: MessageValidity,
    ) -> arbitrary::Result<
        GeneratedMessage<S, <R as DispatchCall>::Decodable, BasicChangeLogEntry<S>>,
    > {
        let value = u32::arbitrary(u)?;
        let message = sov_value_setter::CallMessage::SetValueAndSleep {
            value,
            sleep_millis: self.sleep_millis,
        };

        Ok(GeneratedMessage::new(
            <R as EncodeCall<ValueSetter<S>>>::to_decodable(message),
            self.admin_key.clone(),
            MessageOutcome::Successful {
                changes: vec![BasicChangeLogEntry::ValueSetter(
                    ValueSetterChangeLogEntry::ValueUpdated { new_value: value },
                )],
            },
        ))
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

async fn prepare_and_send_txs<R: Runtime<S> + Clone, S: Spec>(
    modules: Vec<BasicModuleRef<S, R>>,
    client: &sov_api_spec::Client,
    rx: Receiver<bool>,
    worker_id: u128,
    num_workers: u32,
    validity: Distribution<MessageValidity>,
    use_retries: bool,
) -> anyhow::Result<()> {
    start_tx_attempt_stats_logger();

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
            let message_validity = if *validity == MessageValidity::Invalid {
                MessageValidity::Valid
            } else {
                *validity
            };
            let msg = generator.generate(&modules, message_validity);
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

                sign_with_current_generation::<R, S>(message, key, details)?
            };

            if *validity == MessageValidity::Invalid {
                txns.push((signed_tx.clone(), false));
                txns.push((signed_tx, true));
                continue;
            }

            txns.push((signed_tx, false));
        }

        let start = std::time::Instant::now();
        for (tx, is_invalid) in &txns {
            if *is_invalid {
                match client.send_tx_to_sequencer(tx).await {
                    Ok(_) => {
                        record_tx_attempt(true);
                        tracing::debug!(
                            id = %worker_id,
                            attempt = 1,
                            expected_rejection = true,
                            "Transaction accepted"
                        );
                        anyhow::bail!("Duplicate generation transaction should have failed");
                    }
                    Err(err) => {
                        record_tx_attempt(false);
                        tracing::debug!(
                            id = %worker_id,
                            attempt = 1,
                            expected_rejection = true,
                            ?err,
                            "Transaction rejected"
                        );
                        if sov_api_spec::is_stop_height_error(&err) {
                            tracing::info!(
                                "Sequencer reached stop height, gracefully stopping soak worker"
                            );
                            return Ok(());
                        }
                    }
                }
            } else {
                let max_attempts = if use_retries {
                    TX_SEND_RETRY_ATTEMPTS
                } else {
                    1
                };

                for attempt in 1..=max_attempts {
                    match client.send_tx_to_sequencer(tx).await {
                        Ok(_) => {
                            record_tx_attempt(true);
                            tracing::debug!(
                                id = %worker_id,
                                attempt,
                                expected_rejection = false,
                                "Transaction accepted"
                            );
                            break;
                        }
                        Err(err) => {
                            record_tx_attempt(false);
                            tracing::debug!(
                                id = %worker_id,
                                attempt,
                                expected_rejection = false,
                                ?err,
                                "Transaction rejected"
                            );
                            if sov_api_spec::is_stop_height_error(&err) {
                                tracing::info!(
                                    "Sequencer reached stop height, gracefully stopping soak worker"
                                );
                                return Ok(());
                            }

                            if attempt == max_attempts {
                                return Err(err.into());
                            }

                            tokio::time::sleep(TX_SEND_RETRY_DELAY).await;
                        }
                    }
                }
            }
            total_txns += 1;
        }
        let elapsed = start.elapsed();
        tracing::debug!(id = %worker_id, "Sent {} transactions in {}ms. Current throughput: {:.2} txs per second. Running throughput: {:.2} txs per second", txns.len(), elapsed.as_millis(), (txns.len() * num_workers as usize) as f64 / elapsed.as_secs_f64(), (total_txns * num_workers as usize) as f64 / worker_start.elapsed().as_secs_f64());
    }

    Ok(())
}
