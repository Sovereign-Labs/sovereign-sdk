use std::cmp::max;
use std::collections::HashSet;

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
        let user_session = self.state_session_builder.begin_user_session().unwrap();
        let kernel_session = self.state_session_builder.begin_kernel_session().unwrap();

        (user_session, kernel_session)
    }
}

#[cfg(test)]
impl crate::storage_manager::InitializableNativeNomtStorage<H, SlotHash> for TestNomtStorage {
    fn new(
        state_session_builder: crate::state_db_nomt::NomtSessionBuilder<H, SlotHash>,
        historical_state: crate::historical_state::HistoricalStateReader,
        accessory_db: AccessoryDb,
        _use_strict_mode: bool,
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
