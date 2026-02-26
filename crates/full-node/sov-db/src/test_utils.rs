#[cfg(any(test, feature = "test-utils"))]
use std::any::Any;
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

#[cfg(any(test, feature = "test-utils"))]
#[allow(missing_docs)]
pub type H = sha2::Sha256;
#[cfg(any(test, feature = "test-utils"))]
/// Default slot hash for tests.
pub type SlotHash = sov_mock_da::MockHash;

/// Simple container fo unlocking testing of NomtStorageManager without relying on sov-state.
#[cfg(any(test, feature = "test-utils"))]
#[allow(missing_docs)]
pub struct TestNomtStorage {
    pub state_session_builder: crate::state_db_nomt::NomtSessionBuilder<H, SlotHash>,
    pub historical_state: crate::historical_state::HistoricalStateReader,
    pub accessory_db: AccessoryDb,
}

#[cfg(any(test, feature = "test-utils"))]
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

#[cfg(any(test, feature = "test-utils"))]
impl crate::storage_manager::InitializableNativeNomtStorage<H, SlotHash> for TestNomtStorage {
    fn new(
        state_session_builder: crate::state_db_nomt::NomtSessionBuilder<H, SlotHash>,
        historical_state: crate::historical_state::HistoricalStateReader,
        accessory_db: AccessoryDb,
        _with_witness: bool,
        _pinned_cache: Option<Box<dyn Any + Send + Sync>>,
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

// ---------------------------------------------------------------------------
// Fork-map types (used by storage_manager tests and benchmarks)
// ---------------------------------------------------------------------------

use std::collections::{HashMap, VecDeque};

#[cfg(any(test, feature = "test-utils"))]
use sov_mock_da::{MockBlockHeader, MockHash};
#[cfg(any(test, feature = "test-utils"))]
use sov_rollup_interface::da::BlockHeaderTrait;

/// A description of a fork tree, used to generate deterministic blockchain structures.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Clone, Debug)]
pub struct ForkDescription {
    /// Relative to parent.
    #[allow(missing_docs)]
    pub start_height: u64,
    /// How many blocks this fork has.
    #[allow(missing_docs)]
    pub length: u8,
    /// All forks that start from given Fork.
    /// Note: `start_height` of the child's fork should be below the length of the parent.
    #[allow(missing_docs)]
    pub child_forks: Vec<ForkDescription>,
}

#[cfg(any(test, feature = "test-utils"))]
#[derive(Clone, Debug)]
struct ForkMapBlockInfo {
    header: MockBlockHeader,
    forks: Vec<MockHash>,
    parent: Option<MockHash>,
}

/// It is "materialized" [`ForkDescription`] with all blocks pointing to each other.
/// Can be used in actual tests, guaranteeing the correct blockchain.
#[cfg(any(test, feature = "test-utils"))]
#[derive(Clone, Default, Debug)]
pub struct ForkMap {
    blocks: HashMap<MockHash, ForkMapBlockInfo>,
}

#[cfg(any(test, feature = "test-utils"))]
impl ForkMap {
    #[allow(missing_docs)]
    pub fn get_start(&self) -> Option<MockHash> {
        // Start from any, all should traverse back to original start
        let mut start = self.blocks.keys().next();
        while let Some(block_hash) = start {
            let block = self.blocks.get(block_hash).unwrap();
            if block.parent.is_none() {
                break;
            }
            start = block.parent.as_ref();
        }

        start.cloned()
    }

    #[allow(missing_docs)]
    pub fn get_block_header(&self, block_hash: &MockHash) -> Option<&MockBlockHeader> {
        self.blocks.get(block_hash).map(|i| &i.header)
    }

    #[allow(missing_docs)]
    pub fn blocks_count(&self) -> usize {
        self.blocks.len()
    }

    #[allow(missing_docs)]
    pub fn get_child_hashes(&self, block_hash: &MockHash) -> Vec<MockHash> {
        self.blocks
            .get(block_hash)
            .cloned()
            .map(|x| x.forks)
            .unwrap_or_default()
    }

    /// Builds a whole chain of block headers, up to given hash (inclusive)
    pub fn get_chain_up_to(&self, up_to: MockBlockHeader) -> Vec<MockBlockHeader> {
        let mut chain = Vec::with_capacity(up_to.height() as usize);

        let mut current_hash = up_to.hash();
        while let Some(block_info) = self.blocks.get(&current_hash) {
            chain.push(block_info.header.clone());
            if let Some(parent_hash) = block_info.parent.as_ref() {
                current_hash = *parent_hash;
            } else {
                break;
            }
        }

        chain.reverse();
        chain
    }

    fn insert_to_parent(&mut self, prev_hash: MockHash, current_hash: MockHash) {
        if let std::collections::hash_map::Entry::Vacant(_) = self
            .blocks
            .entry(prev_hash)
            .and_modify(|parent_block_info| parent_block_info.forks.push(current_hash))
        {
            panic!("Parent should be always inserted before updating its forks");
        }
    }
}

#[cfg(any(test, feature = "test-utils"))]
impl From<ForkDescription> for ForkMap {
    fn from(fork: ForkDescription) -> Self {
        let mut chain_map = ForkMap::default();
        let mut forks_to_process = VecDeque::new();
        // starting height, parent_fork_id and original fork.
        forks_to_process.push_back((0u64, 0u64, fork));

        // For debugging purposes.
        let mut nodes_count_1: usize = 0;

        // Flat ids of all forks
        let mut next_fork_id = 0;

        while let Some((height, parent_fork_id, fork)) = forks_to_process.pop_front() {
            next_fork_id += 1;
            let fork_id = next_fork_id;
            nodes_count_1 += fork.length as usize;
            let fork_total_height = height + fork.start_height;
            let prev_hash =
                get_block_hash(parent_fork_id, fork_total_height.checked_sub(1).unwrap());
            let parent = if height > 0 { Some(prev_hash) } else { None };
            let current_hash = get_block_hash(fork_id, fork_total_height);
            let fork_start = MockBlockHeader {
                prev_hash,
                hash: current_hash,
                height: fork_total_height,
                ..Default::default()
            };
            let fork_start = ForkMapBlockInfo {
                header: fork_start,
                forks: Vec::new(),
                parent,
            };
            chain_map.blocks.insert(current_hash, fork_start);
            if height > 0 {
                chain_map.insert_to_parent(prev_hash, current_hash);
            }
            for child_rel_height in 1..fork.length {
                let child_total_height = fork_total_height + child_rel_height as u64;
                let prev_hash = get_block_hash(fork_id, child_total_height - 1);
                let current_hash = get_block_hash(fork_id, child_total_height);
                chain_map.insert_to_parent(prev_hash, current_hash);
                let block_header = MockBlockHeader {
                    prev_hash,
                    hash: current_hash,
                    height: child_total_height,
                    ..Default::default()
                };
                let block_info = ForkMapBlockInfo {
                    header: block_header,
                    forks: Vec::new(),
                    parent: Some(prev_hash),
                };
                if let Some(b) = chain_map.blocks.insert(current_hash, block_info) {
                    panic!("duplicate block on fork_id={fork_id} on {current_hash} from={b:?}");
                };
            }
            for child_fork in fork.child_forks {
                forks_to_process.push_back((fork_total_height, fork_id, child_fork));
            }
        }
        assert_eq!(nodes_count_1, chain_map.blocks.len());
        chain_map
    }
}

/// Gets deterministic MockHash for given height and `fork_id`.
#[cfg(any(test, feature = "test-utils"))]
pub fn get_block_hash(fork_id: u64, height: u64) -> MockHash {
    let mut raw_hash: [u8; 32] = [0; 32];
    let fork_id_bytes = fork_id.to_be_bytes();
    raw_hash[..fork_id_bytes.len()].copy_from_slice(&fork_id_bytes);
    let height_bytes = height.to_be_bytes();
    raw_hash[fork_id_bytes.len()..fork_id_bytes.len() + height_bytes.len()]
        .copy_from_slice(&height_bytes);
    MockHash(raw_hash)
}

// ---------------------------------------------------------------------------
// Data helper functions (used by storage_manager tests and benchmarks)
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "test-utils"))]
use rockbound::cache::delta_reader::DeltaReader;
#[cfg(any(test, feature = "test-utils"))]
use sov_rollup_interface::stf::StoredEvent;

#[cfg(any(test, feature = "test-utils"))]
use crate::schema::tables::EventByNumber;
#[cfg(any(test, feature = "test-utils"))]
use crate::schema::types::slot_key::SlotKey;
#[cfg(any(test, feature = "test-utils"))]
use crate::schema::types::EventNumber;

#[cfg(any(test, feature = "test-utils"))]
fn decode_ledger_item(item: (EventNumber, StoredEvent)) -> (u64, MockHash) {
    let (event_number, stored_event) = item;
    let height = event_number.0;
    assert_eq!(stored_event.key().inner(), stored_event.value().inner());
    let da_hash = MockHash::try_from(stored_event.key().inner().to_vec()).unwrap();
    (height, da_hash)
}

/// Using [`MockBlockHeader::height`] as a key and [`MockHash`] of header as value.
///
/// What it writes:
///  - [`EventByNumber`] => `block_height` => Event {key == block_hash, value == block_hash}
///
/// So it can be validated by traversing data from DB.
#[cfg(any(test, feature = "test-utils"))]
pub fn materialize_ledger_changes(da_header: &MockBlockHeader) -> SchemaBatch {
    let mut change_set = SchemaBatch::default();
    let key = &EventNumber(da_header.height());
    let value = StoredEvent::new(&da_header.hash().0, &da_header.hash().0, da_header.hash().0);

    change_set.put::<EventByNumber>(key, &value).unwrap();

    change_set
}

#[allow(missing_docs)]
#[cfg(any(test, feature = "test-utils"))]
pub fn verify_accessory_db(
    accessory_db: &crate::accessory_db::AccessoryDb,
    expected_values: &[(u64, MockHash)],
) {
    for (expected_height, expected_hash) in expected_values {
        let key = SlotKey::from_slice(&expected_height.to_be_bytes());
        let actual_value = accessory_db
            .get_value_option(&key, SlotNumber::GENESIS)
            .unwrap()
            .expect("Missing value in AccessoryDb");
        assert_eq!(expected_hash.0.to_vec(), actual_value);
    }
}

#[allow(missing_docs)]
#[cfg(any(test, feature = "test-utils"))]
pub fn verify_ledger_storage(reader: &DeltaReader, expected_values: &[(u64, MockHash)]) {
    let range = EventNumber(0)..EventNumber(u64::MAX);

    let actual_values: Vec<(u64, MockHash)> = reader
        .collect_in_range::<EventByNumber, _>(range)
        .unwrap()
        .into_iter()
        .map(decode_ledger_item)
        .collect();
    assert_eq!(expected_values, &actual_values);
}

#[allow(missing_docs)]
#[cfg(any(test, feature = "test-utils"))]
pub fn get_expected_chain_values(processed_chain: &[MockBlockHeader]) -> Vec<(u64, MockHash)> {
    processed_chain
        .iter()
        .map(|b| (b.height(), b.hash()))
        .collect()
}

use strum::{Display, EnumString};

/// This environment variable sets the crash location for rollup and is used only in tests.
pub const CRASH_ENV_NAME: &str = "SOV_CRASH_ON_COMMIT";

/// The crash location.
#[derive(Debug, Clone, Display, EnumString, Eq, PartialEq)]
pub enum CrashLocation {
    /// Rollup crashes before committing the kernel.
    BeforeCommittingKernelNomt,
    /// Rollup crashes before committing the user nomt.
    BeforeCommittingUserNomt,
    /// Rollup crashes before committing the ledger.
    BeforeCommittingLedger,
    /// Rollup crashes before committing the accessory.
    BeforeCommittingAccessory,
    /// Rollup crashes before committing the the archival db.
    BeforeCommittingArchival,
    /// Rollup crashes before committing the the live db.
    BeforeCommittingLive,
}

impl CrashLocation {
    /// Sets `CRASH_ENV_NAME` to `self`.
    pub fn set_crash_env(&self) {
        std::env::set_var(CRASH_ENV_NAME, self.to_string());
    }

    /// if `CRASH_ENV_NAME` is set to self, the method will panic.
    pub fn crash_if_env_set(&self) {
        if cfg!(debug_assertions) {
            if let Ok(env) = std::env::var(CRASH_ENV_NAME) {
                let crash_location: CrashLocation = env.parse().unwrap();

                if &crash_location == self {
                    tracing::error!(
                        "{CRASH_ENV_NAME} is set to: {crash_location}, crashing the node"
                    );
                    panic!("{CRASH_ENV_NAME} is set to: {crash_location}, crashing the node");
                }
            }
        }
    }
}
