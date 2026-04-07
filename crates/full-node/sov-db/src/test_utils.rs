use std::cmp::max;
use std::collections::HashSet;
#[cfg(any(test, feature = "test-utils"))]
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::Duration;

use jmt::{JellyfishMerkleTree, KeyHash, SimpleHasher};
use rand::{Rng, SeedableRng};
use rockbound::{SchemaBatch, SchemaValue};
use sov_rollup_interface::common::SlotNumber;

use crate::accessory_db::AccessoryDb;
use crate::namespaces::Namespace;
use crate::schema::tables::ModuleAccessoryState;
use crate::state_db::{JmtHandler, StateDb, StateTreeChanges};
use crate::storage_manager::InitializableNativeStorage;

/// Simple container for unlocking testing of NativeStorage without need of ProverStorage.
#[derive(Debug, Clone)]
pub struct TestNativeStorage {
    #[allow(missing_docs)]
    pub state: StateDb,
    #[allow(missing_docs)]
    pub accessory_db: AccessoryDb,
}

impl InitializableNativeStorage for TestNativeStorage {
    fn new(db: StateDb, accessory_db: AccessoryDb) -> Self {
        Self {
            state: db,
            accessory_db,
        }
    }
}

#[cfg(test)]
#[allow(missing_docs)]
pub type H = sha2::Sha256;
#[cfg(test)]
/// Default slot hash for tests.
pub type SlotHash = sov_mock_da::MockHash;

/// Simple container fo unlocking testing of NomtStorageManager without relying on sov-state.
#[cfg(test)]
#[allow(missing_docs)]
pub struct TestNomtStorage {
    pub state_session_builder: crate::state_db_nomt::NomtSessionBuilder<H, SlotHash>,
    pub historical_state: crate::historical_state::HistoricalStateReader,
    pub accessory_db: AccessoryDb,
}

#[cfg(test)]
impl TestNomtStorage {
    #[allow(missing_docs)]
    pub fn begin_sessions(
        &self,
    ) -> (
        nomt::Session<nomt::hasher::BinaryHasher<H>>,
        nomt::Session<nomt::hasher::BinaryHasher<H>>,
    ) {
        let user_session = self
            .state_session_builder
            .begin_user_session_without_witness()
            .unwrap();
        let kernel_session = self
            .state_session_builder
            .begin_kernel_session_without_witness()
            .unwrap();

        (user_session, kernel_session)
    }
}

#[cfg(test)]
impl crate::storage_manager::InitializableNativeNomtStorage<H, SlotHash> for TestNomtStorage {
    fn new(
        state_session_builder: crate::state_db_nomt::NomtSessionBuilder<H, SlotHash>,
        historical_state: crate::historical_state::HistoricalStateReader,
        accessory_db: AccessoryDb,
        _strict_mode: bool,
        _witness_mode: crate::storage_manager::WitnessMode,
    ) -> Self {
        TestNomtStorage {
            state_session_builder,
            historical_state,
            accessory_db,
        }
    }
}

#[allow(missing_docs)]
pub fn generate_random_bytes(count: usize) -> HashSet<Vec<u8>> {
    let seed: [u8; 32] = [1; 32];

    // Create an RNG with the specified seed, so tests are reproducible.
    // We don't need actual randomness, we need some value distribution.
    let mut rng = rand::prelude::StdRng::from_seed(seed);

    generate_more_random_bytes(&mut rng, count, &HashSet::new())
}

/// Generates more unique keys, which are also not present in given keys.
pub fn generate_more_random_bytes<R: Rng>(
    rng: &mut R,
    count: usize,
    existing_keys: &HashSet<Vec<u8>>,
) -> HashSet<Vec<u8>> {
    let mut samples: HashSet<Vec<u8>> = HashSet::with_capacity(count);

    while samples.len() < count {
        let inner_vec_size = rng.gen_range(32..=256);
        let storage_key: Vec<u8> = (0..inner_vec_size).map(|_| rng.gen::<u8>()).collect();
        if !existing_keys.contains(&storage_key) {
            samples.insert(storage_key);
        }
    }
    samples
}

/// Helper for building proper [`StateTreeChanges`]
pub fn build_data_to_materialize<N: Namespace, H: SimpleHasher>(
    jmt_handler: &JmtHandler<N>,
    next_version: jmt::Version,
    batch: Vec<(KeyHash, Option<SchemaValue>)>,
) -> StateTreeChanges {
    let jmt = JellyfishMerkleTree::<JmtHandler<N>, H>::new(jmt_handler);
    let (_new_root, _update_proof, tree_update) =
        jmt.put_value_set_with_proof(batch, next_version).unwrap();

    StateTreeChanges {
        original_write_values: tree_update.node_batch.values().clone(),
        node_batch: tree_update.node_batch,
    }
}

/// Describes how versions should be distributed across keys.
/// Used for benchmarking pruner.
pub enum VersionDistribution {
    /// All keys have exactly the same number of versions
    #[allow(missing_docs)]
    Uniform { versions_per_key: usize },
    /// Different percentages of keys have different version counts
    /// Vec<(percentage, version_count)> - percentages should sum to ~1.0
    Distributed {
        /// Each profile describes the percentage of keys and their version count.
        /// All profiles should sum to ~1.0.
        profiles: Vec<(f64, usize)>,
    },
    /// Random distribution within a range
    Random {
        #[allow(missing_docs)]
        min_versions: usize,
        #[allow(missing_docs)]
        max_versions: usize,
    },
}

impl VersionDistribution {
    fn max_version(&self) -> usize {
        match self {
            VersionDistribution::Uniform { versions_per_key } => *versions_per_key,
            VersionDistribution::Distributed { profiles } => {
                let max_version = profiles.iter().map(|(_, count)| *count).max().unwrap();
                max(max_version, 1)
            }
            VersionDistribution::Random { max_versions, .. } => *max_versions,
        }
    }

    /// How many versions should a given key have based on its index.
    #[allow(clippy::float_arithmetic)]
    fn key_version_count(&self, key_index: usize, total_unique_keys: usize) -> usize {
        match self {
            VersionDistribution::Uniform { versions_per_key } => *versions_per_key,
            VersionDistribution::Distributed { profiles } => {
                let mut cumulative_percentage = 0.0;
                let key_bucket = key_index as f64 / total_unique_keys as f64;
                if key_bucket > 1.0 {
                    panic!("Key bucket should be less than 1.0");
                }

                for (percentage, version_count) in profiles {
                    cumulative_percentage += percentage;
                    if key_bucket <= cumulative_percentage {
                        return *version_count;
                    }
                }

                // Fallback to last profile.
                profiles.last().map(|(_, count)| *count).unwrap_or(1)
            }
            VersionDistribution::Random {
                min_versions,
                max_versions,
            } => {
                // Use deterministic random based on key_index for reproducibility
                let mut key_rng = rand::prelude::StdRng::seed_from_u64(key_index as u64 + 42);
                key_rng.gen_range(*min_versions..=*max_versions)
            }
        }
    }
}

/// Fill accessory DB with data based on given distribution.
pub fn fill_accessory_db(
    rocksdb: &rockbound::DB,
    total_unique_keys: usize,
    distribution: VersionDistribution,
    start_slot_number: Option<SlotNumber>,
    key_prefix: &str,
) -> anyhow::Result<()> {
    // How often write data to disk
    const BATCH_SIZE: usize = 10_000;

    let max_version = distribution.max_version();
    let start_slot = start_slot_number.unwrap_or(SlotNumber::GENESIS);
    let end_slot = start_slot.checked_add(max_version as u64).unwrap();

    let mut batch = SchemaBatch::new();
    let mut current_batch_size = 0;

    for current_slot in start_slot.get()..=end_slot.get() {
        let current_slot = SlotNumber::new(current_slot);
        let version_offset = (current_slot.get() - start_slot.get()) as usize;

        for key_index in 0..total_unique_keys {
            let key_version_count = distribution.key_version_count(key_index, total_unique_keys);

            // Deterministically select which versions this key should have
            let should_have_data = if key_version_count >= max_version {
                // If key should have all or more versions, it has data at every slot
                true
            } else {
                // Use deterministic approach: check if this version_offset is one of the
                // selected slots for this key by simulating the random selection process
                // and checking if we would have selected this specific offset
                is_version_selected_for_key(
                    key_index,
                    version_offset,
                    key_version_count,
                    max_version,
                )
            };

            if !should_have_data {
                continue;
            }
            let key = format!("{key_prefix}{key_index}").into_bytes();
            let value = Some(format!("value_{}_{}", key_index, current_slot.get()).into_bytes());

            batch.put::<ModuleAccessoryState>(&(key.clone(), current_slot), &value)?;
            current_batch_size += 1;

            if current_batch_size >= BATCH_SIZE {
                rocksdb.write_schemas(&batch)?;
                batch = SchemaBatch::new();
                current_batch_size = 0;
            }
        }
    }

    // Write remaining entries
    if current_batch_size > 0 {
        rocksdb.write_schemas(&batch)?;
    }

    Ok(())
}

/// Deterministically check if a given version_offset should be selected for a key
/// Uses a hash-based approach to avoid generating the full set of selected versions
#[allow(clippy::float_arithmetic)]
fn is_version_selected_for_key(
    key_index: usize,
    version_offset: usize,
    key_version_count: usize,
    max_version: usize,
) -> bool {
    // Calculate the probability that this version should be selected
    let selection_probability = key_version_count as f64 / max_version as f64;

    // Use a deterministic hash-based approach
    // Combine key_index and version_offset to create a unique seed for this specific check
    let hash_seed = (key_index as u64) + 42 + (version_offset as u64);
    let mut rng = rand::prelude::StdRng::seed_from_u64(hash_seed);

    // Generate a random float and compare against the probability
    let random_value: f64 = rng.gen();

    // Adjust probability to ensure we get approximately the right number of versions
    // This is an approximation, but much more efficient than the exact method
    random_value < selection_probability
}

use strum::{Display, EnumString};

/// This environment variable selects a fault injection location that should panic and is used only
/// in tests.
pub const CRASH_ON_COMMIT_ENV_NAME: &str = "SOV_CRASH_ON_COMMIT";
/// This environment variable configures a sleep fault in the form `LOCATION:TIME_MS`.
pub const SLEEP_ON_COMMIT_MS_ENV_NAME: &str = "SOV_SLEEP_ON_COMMIT_MS";

/// A commit lifecycle location where a test-only fault can be injected.
#[derive(Debug, Clone, Display, EnumString, Eq, PartialEq)]
pub enum CommitFaultInjectionLocation {
    /// Inject an action before committing the kernel NOMT state.
    BeforeCommittingKernelNomt,
    /// Inject an action before committing the user NOMT state.
    BeforeCommittingUserNomt,
    /// Inject an action before committing the ledger.
    BeforeCommittingLedger,
    /// Inject an action before committing the accessory state.
    BeforeCommittingAccessory,
    /// Inject an action before committing the archival db.
    BeforeCommittingArchival,
    /// Inject an action before committing the live db.
    BeforeCommittingLive,
}

#[cfg(any(test, feature = "test-utils"))]
#[derive(Debug, Default)]
struct CommitFaultWaitState {
    armed_location: Option<CommitFaultInjectionLocation>,
    reached: bool,
    released: bool,
}

#[cfg(any(test, feature = "test-utils"))]
static COMMIT_FAULT_WAIT_GATE: LazyLock<(Mutex<CommitFaultWaitState>, Condvar)> =
    LazyLock::new(|| (Mutex::new(CommitFaultWaitState::default()), Condvar::new()));

impl CommitFaultInjectionLocation {
    /// Sets the crash env var to `self`.
    pub fn set_crash_env(&self) {
        std::env::set_var(CRASH_ON_COMMIT_ENV_NAME, self.to_string());
    }

    /// Sets the sleep env var to `LOCATION:TIME_MS`.
    pub fn set_sleep_env(&self, ms: u64) {
        std::env::set_var(SLEEP_ON_COMMIT_MS_ENV_NAME, format!("{self}:{ms}"));
    }

    #[cfg(any(test, feature = "test-utils"))]
    /// Arms the in-process wait gate for this location.
    pub fn arm_wait_gate(&self) {
        let (lock, _) = &*COMMIT_FAULT_WAIT_GATE;
        let mut state = lock.lock().expect("Commit fault wait gate lock poisoned");
        assert!(
            state.armed_location.is_none(),
            "Commit fault wait gate is already armed for {:?}",
            state.armed_location
        );
        *state = CommitFaultWaitState {
            armed_location: Some(self.clone()),
            reached: false,
            released: false,
        };
    }

    #[cfg(any(test, feature = "test-utils"))]
    /// Waits for the in-process wait gate for this location to be reached.
    pub fn wait_until_wait_gate_reached(&self, timeout: Duration) -> bool {
        let (lock, cv) = &*COMMIT_FAULT_WAIT_GATE;
        let state = lock.lock().expect("Commit fault wait gate lock poisoned");
        assert_eq!(
            state.armed_location.as_ref(),
            Some(self),
            "Commit fault wait gate is not armed for {self}"
        );
        let (state, _) = cv
            .wait_timeout_while(state, timeout, |state| !state.reached)
            .expect("Commit fault wait gate lock poisoned");
        state.reached
    }

    #[cfg(any(test, feature = "test-utils"))]
    /// Releases the in-process wait gate for this location and waits until it is cleared.
    pub fn release_wait_gate(&self) {
        let (lock, cv) = &*COMMIT_FAULT_WAIT_GATE;
        let mut state = lock.lock().expect("Commit fault wait gate lock poisoned");
        assert_eq!(
            state.armed_location.as_ref(),
            Some(self),
            "Commit fault wait gate is not armed for {self}"
        );
        assert!(
            state.reached,
            "Commit fault wait gate for {self} was not reached"
        );
        state.released = true;
        cv.notify_all();
        while state.armed_location.is_some() {
            state = cv
                .wait(state)
                .expect("Commit fault wait gate lock poisoned");
        }
    }

    /// If a fault is configured for `self`, this method injects it.
    pub fn inject_fault_if_configured(&self) {
        #[cfg(any(test, feature = "test-utils"))]
        self.block_on_wait_gate_if_armed();

        if self.is_crash_env_set() {
            tracing::error!("{CRASH_ON_COMMIT_ENV_NAME} is set to: {self}, crashing the node");
            panic!("{CRASH_ON_COMMIT_ENV_NAME} is set to: {self}, crashing the node");
        }

        if let Some(duration) = self.sleep_duration_from_env() {
            tracing::warn!(
                location = %self,
                sleep_ms = duration.as_millis(),
                "{SLEEP_ON_COMMIT_MS_ENV_NAME} matched; pausing before commit"
            );
            std::thread::sleep(duration);
        }
    }

    /// Returns true if the crash env var is set to this location.
    ///
    /// # Panics
    /// Panics if the env var is set to a value that cannot be parsed as a
    /// `CommitFaultInjectionLocation`.
    pub fn is_crash_env_set(&self) -> bool {
        if !cfg!(debug_assertions) {
            return false;
        }
        match std::env::var(CRASH_ON_COMMIT_ENV_NAME) {
            Ok(env) => {
                let injection_location: CommitFaultInjectionLocation =
                    env.parse().unwrap_or_else(|e| {
                    panic!(
                        "Failed to parse {CRASH_ON_COMMIT_ENV_NAME}={env:?} as CommitFaultInjectionLocation: {e}"
                    )
                });
                &injection_location == self
            }
            Err(_) => false,
        }
    }

    fn sleep_duration_from_env(&self) -> Option<Duration> {
        if !cfg!(debug_assertions) {
            return None;
        }

        match std::env::var(SLEEP_ON_COMMIT_MS_ENV_NAME) {
            Ok(env) => {
                let (location, sleep_ms) = env.split_once(':').unwrap_or_else(|| {
                    panic!(
                        "Failed to parse {SLEEP_ON_COMMIT_MS_ENV_NAME}={env:?} as LOCATION:TIME_MS"
                    )
                });

                let injection_location: CommitFaultInjectionLocation =
                    location.parse().unwrap_or_else(|e| {
                        panic!(
                            "Failed to parse location in {SLEEP_ON_COMMIT_MS_ENV_NAME}={env:?} as CommitFaultInjectionLocation: {e}"
                        )
                    });
                if &injection_location != self {
                    return None;
                }

                let sleep_ms = sleep_ms.parse::<u64>().unwrap_or_else(|e| {
                    panic!(
                        "Failed to parse duration in {SLEEP_ON_COMMIT_MS_ENV_NAME}={env:?} as u64: {e}"
                    )
                });
                Some(Duration::from_millis(sleep_ms))
            }
            Err(_) => None,
        }
    }

    #[cfg(any(test, feature = "test-utils"))]
    fn block_on_wait_gate_if_armed(&self) {
        let (lock, cv) = &*COMMIT_FAULT_WAIT_GATE;
        let mut state = lock.lock().expect("Commit fault wait gate lock poisoned");
        if state.armed_location.as_ref() != Some(self) {
            return;
        }

        state.reached = true;
        cv.notify_all();
        while !state.released {
            state = cv
                .wait(state)
                .expect("Commit fault wait gate lock poisoned");
        }

        *state = CommitFaultWaitState::default();
        cv.notify_all();
    }
}

#[cfg(test)]
mod commit_fault_injection_tests {
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    use super::{
        CommitFaultInjectionLocation, CRASH_ON_COMMIT_ENV_NAME, SLEEP_ON_COMMIT_MS_ENV_NAME,
    };

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn inject_fault_if_configured_sleeps_when_configured() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(CRASH_ON_COMMIT_ENV_NAME);
        std::env::remove_var(SLEEP_ON_COMMIT_MS_ENV_NAME);

        CommitFaultInjectionLocation::BeforeCommittingLedger.set_sleep_env(20);

        let start = Instant::now();
        CommitFaultInjectionLocation::BeforeCommittingLedger.inject_fault_if_configured();
        assert!(start.elapsed() >= Duration::from_millis(20));

        std::env::remove_var(CRASH_ON_COMMIT_ENV_NAME);
        std::env::remove_var(SLEEP_ON_COMMIT_MS_ENV_NAME);
    }

    #[test]
    fn inject_fault_if_configured_crash_takes_precedence_over_sleep() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var(CRASH_ON_COMMIT_ENV_NAME);
        std::env::remove_var(SLEEP_ON_COMMIT_MS_ENV_NAME);

        let location = CommitFaultInjectionLocation::BeforeCommittingLedger;
        location.set_crash_env();
        location.set_sleep_env(20);

        let result = std::panic::catch_unwind(|| {
            location.inject_fault_if_configured();
        });
        assert!(result.is_err());

        std::env::remove_var(CRASH_ON_COMMIT_ENV_NAME);
        std::env::remove_var(SLEEP_ON_COMMIT_MS_ENV_NAME);
    }
}
