use std::path::Path;
use std::sync::{Arc, Mutex};

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::versioned_db::VersionedDeltaReader;
use rockbound::SchemaBatch;
use sov_db::accessory_db::AccessoryDb;
use sov_db::config::RollupDbConfig;
use sov_db::historical_state::HistoricalStateReader;
use sov_db::ledger_db::LedgerDb;
use sov_db::namespaces::{KernelNamespace, UserNamespace};
use sov_db::schema::namespace::NomtStateValues;
use sov_db::state_db_nomt::get_session_builder_from_committed;
use sov_db::storage_manager::{FlatStateDb, InitializableNativeNomtStorage, WitnessMode};
pub use sov_db::storage_manager::{NomtChangeSet, NomtStorageManager};
use sov_db::DbCache;
use sov_mock_da::{MockBlockHeader, MockDaSpec};
use sov_modules_api::digest;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{MerkleProofSpec, NativeStorage, Storage, StorageRoot};
use tempfile::TempDir;

use crate::TestSlotHash;

/// Storage manager suitable for [`LedgerDb`].
pub struct SimpleLedgerStorageManager {
    db: Arc<rockbound::DB>,
}

impl SimpleLedgerStorageManager {
    /// Initialize a new instance in the given path.
    pub fn new(path: impl AsRef<std::path::Path>) -> Self {
        let db = LedgerDb::get_rockbound_options()
            .default_setup_db_as_subdir(path.as_ref())
            .unwrap();
        Self { db: Arc::new(db) }
    }

    /// Initialize a new instance at an unspecified path.
    pub fn new_any_path() -> Self {
        let dir = tempfile::tempdir().unwrap();
        Self::new(dir.path())
    }

    /// Create the new [`DeltaReader`] which has visibility only on persisted changes.
    pub fn create_ledger_storage(&mut self) -> DeltaReader {
        DeltaReader::new(self.db.clone(), Vec::new())
    }

    /// Write changes directly to the underlying db
    pub fn commit(&mut self, ledger_change_set: &SchemaBatch) {
        self.db.write_schemas(ledger_change_set).unwrap();
    }

    /// Get a reference to the underlying database
    pub fn get_db(&self) -> Arc<rockbound::DB> {
        self.db.clone()
    }
}

/// Implementation of [`HierarchicalStorageManager`] that provides [`NomtProverStorage`]
/// and commits changes directly to the underlying database.
pub struct SimpleStorageManager<S: MerkleProofSpec> {
    // Holds ownership of [`Tempdir`] so it is not removed prematurely
    _dir: TempDir,
    state: Arc<sov_db::state_db_nomt::NomtStateDb<S::Hasher>>,
    historical_state: FlatStateDb,
    accessory: Arc<rockbound::DB>,
    root: StorageRoot<S>,
    is_strict_mode: bool,
    with_witness: bool,
}

impl<S: MerkleProofSpec> SimpleStorageManager<S> {
    /// Initialize a new instance of [`SimpleStorageManager`] in a temporary directory.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = RollupDbConfig::default_in_path(dir.path().to_path_buf());

        let state_db = sov_db::state_db_nomt::NomtStateDb::new(config)
            .expect("Failed to initialize NOMT state DB");
        let historical_state = FlatStateDb::new(dir.path().to_path_buf(), 1_000_000).unwrap(); // Use a 1MB state cache for tests
        let accessory_rocksdb = AccessoryDb::get_rockbound_options()
            .default_setup_db_as_subdir(dir.path())
            .unwrap();

        Self {
            _dir: dir,
            state: Arc::new(state_db),
            historical_state,
            accessory: Arc::new(accessory_rocksdb),
            root: <NomtProverStorage<S, TestSlotHash> as Storage>::PRE_GENESIS_ROOT,
            is_strict_mode: true,
            with_witness: true,
        }
    }

    /// Change in which mode storage is going to be created.
    pub fn set_strict_mode(&mut self, use_strict_mode: bool) {
        self.is_strict_mode = use_strict_mode;
    }

    /// Set witness generation independently from strict mode.
    pub fn set_witness_generation(&mut self, with_witness: bool) {
        self.with_witness = with_witness;
    }

    /// Create a new [`NomtProverStorage`] that has a view only on data written to disc.
    pub fn create_storage(&self) -> NomtProverStorage<S, TestSlotHash> {
        let flat_state = &self.historical_state;
        let other_data_reader =
            DeltaReader::new(self.historical_state.get_db().clone(), Vec::new());
        let version = HistoricalStateReader::last_version_from_reader(&other_data_reader)
            .unwrap()
            .map(|v| v.get());
        let user_state_reader =
            VersionedDeltaReader::<NomtStateValues<UserNamespace>, DbCache>::new(
                flat_state.get_user_db().clone(),
                version,
                vec![],
            );
        let kernel_state_reader =
            VersionedDeltaReader::<NomtStateValues<KernelNamespace>, DbCache>::new(
                flat_state.get_kernel_db().clone(),
                version,
                vec![],
            );

        let state_session_builder = get_session_builder_from_committed(self.state.clone());
        let historical_state_reader =
            HistoricalStateReader::new(user_state_reader, kernel_state_reader, other_data_reader);
        let accessory_db =
            AccessoryDb::with_reader(DeltaReader::new(self.accessory.clone(), Vec::new()))
                .expect("Failed to create accessory db");

        let witness_mode = WitnessMode::from_bool(self.with_witness);
        NomtProverStorage::create(
            state_session_builder,
            historical_state_reader,
            accessory_db,
            self.is_strict_mode,
            witness_mode,
        )
    }

    /// Commit [`NomtChangeSet`] to disk.
    pub fn commit(&mut self, stf_change_set: NomtChangeSet) {
        let NomtChangeSet {
            state,
            historical_state,
            accessory,
        } = stf_change_set;

        self.state.commit_change_set(state).unwrap();
        self.accessory.write_schemas(&accessory).unwrap();
        self.historical_state.commit(historical_state).unwrap();
    }
}

impl<S: MerkleProofSpec> Default for SimpleStorageManager<S> {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait that represents a storage manager that is not attached to a particular DA layer.
/// All changes are linear.
/// Commited data will be available in the following call to create_prover_storage.
#[allow(missing_docs)]
pub trait ForklessStorageManager {
    type Storage: NativeStorage;
    fn new_in_tempdir() -> Self;
    fn current_root(&self) -> <Self::Storage as Storage>::Root;
    fn create_storage_with_root(&self) -> (Self::Storage, <Self::Storage as Storage>::Root) {
        (self.create_prover_storage(), self.current_root())
    }
    fn create_prover_storage(&self) -> Self::Storage;

    fn create_api_storage(&self) -> Self::Storage {
        self.create_prover_storage()
    }

    fn commit_state_update(
        &mut self,
        storage: Self::Storage,
        state_update: <Self::Storage as Storage>::StateUpdate,
        new_root: <Self::Storage as Storage>::Root,
    ) {
        let change_set = storage.materialize_changes(state_update);
        self.commit_change_set(change_set, new_root);
    }
    fn commit_change_set(
        &mut self,
        change_set: <Self::Storage as Storage>::ChangeSet,
        new_root: <Self::Storage as Storage>::Root,
    );
}

impl<S: MerkleProofSpec> ForklessStorageManager for SimpleStorageManager<S> {
    type Storage = NomtProverStorage<S, TestSlotHash>;

    fn new_in_tempdir() -> Self {
        Self::new()
    }

    fn current_root(&self) -> <Self::Storage as Storage>::Root {
        self.root
    }

    fn create_prover_storage(&self) -> Self::Storage {
        self.create_storage()
    }

    fn commit_change_set(
        &mut self,
        change_set: <Self::Storage as Storage>::ChangeSet,
        new_root: <Self::Storage as Storage>::Root,
    ) {
        self.commit(change_set);
        self.root = new_root;
    }
}

/// Allows to initialize it in path!
pub trait PathInitializer {
    #[allow(missing_docs)]
    fn new_in_path(path: impl AsRef<std::path::Path>) -> Self;
}

impl<Da, H, S> PathInitializer for NomtStorageManager<Da, H, S>
where
    Da: DaSpec,
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    S: InitializableNativeNomtStorage<H, Da::SlotHash>,
{
    fn new_in_path(path: impl AsRef<Path>) -> Self {
        let config = RollupDbConfig::default_in_path(path.as_ref().to_path_buf());
        Self::new(config, false).unwrap()
    }
}

/// Using [`HierarchicalStorageManager`] to mimic [`SimpleStorageManager`],
/// but instead of committing all data on disk, it just appends it to the following block.
/// Emulates fork-less DA.
///
/// The `FINALIZE` const generic controls whether blocks are finalized after saving:
/// - `false`: Only saves change sets (emulates DA without finality)
/// - `true`: Saves and finalizes (emulates DA with instant finality)
pub struct HierarchicalForklessStorageManager<
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: Storage,
    const FINALIZE: bool,
> {
    _dir: TempDir,
    // It holds mutex over the inner storage manager, for compatibility with the testing framework.
    storage_manager: Mutex<H>,
    last_block: MockBlockHeader,
    root: S::Root,
}

/// Storage manager that does not finalize blocks - emulates fork-less DA without finality.
pub type NonCommitingStorageManager<H, S> = HierarchicalForklessStorageManager<H, S, false>;

/// Storage manager that finalizes blocks - emulates MockDa with instant finality.
pub type CommitingStorageManager<H, S> = HierarchicalForklessStorageManager<H, S, true>;

impl<H, S, const FINALIZE: bool> HierarchicalForklessStorageManager<H, S, FINALIZE>
where
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: NativeStorage,
{
    /// Create a new [`HierarchicalForklessStorageManager`].
    /// Creates a temporary directory that is kept alive for the lifetime of the manager.
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let storage_manager = H::new_in_path(dir.path());
        let initial_block_header = MockBlockHeader::from_height(0);
        Self {
            _dir: dir,
            storage_manager: Mutex::new(storage_manager),
            last_block: initial_block_header,
            root: S::PRE_GENESIS_ROOT,
        }
    }
}

impl<H, S, const FINALIZE: bool> Default for HierarchicalForklessStorageManager<H, S, FINALIZE>
where
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: NativeStorage,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<H, S, const FINALIZE: bool> ForklessStorageManager
    for HierarchicalForklessStorageManager<H, S, FINALIZE>
where
    H: HierarchicalStorageManager<MockDaSpec, StfState = S, StfChangeSet = S::ChangeSet>
        + PathInitializer,
    <H as HierarchicalStorageManager<MockDaSpec>>::LedgerChangeSet: Default,
    S: NativeStorage,
{
    type Storage = S;

    fn new_in_tempdir() -> Self {
        Self::new()
    }

    fn current_root(&self) -> <Self::Storage as Storage>::Root {
        self.root.clone()
    }

    fn create_prover_storage(&self) -> Self::Storage {
        let (prover_storage, _) = self
            .storage_manager
            .lock()
            .unwrap()
            .create_state_for(&self.last_block)
            .expect("Failed to create storage");
        prover_storage
    }

    fn create_api_storage(&self) -> Self::Storage {
        let (prover_storage, _) = self
            .storage_manager
            .lock()
            .unwrap()
            .create_state_after(&self.last_block)
            .expect("Failed to create storage");
        prover_storage
    }

    fn commit_change_set(
        &mut self,
        change_set: <Self::Storage as Storage>::ChangeSet,
        new_root: <Self::Storage as Storage>::Root,
    ) {
        self.root = new_root;
        let mut storage_manager = self.storage_manager.lock().unwrap();
        storage_manager
            .save_change_set(&self.last_block, change_set, Default::default())
            .expect("Failed to save change set");
        if FINALIZE {
            storage_manager
                .finalize(&self.last_block)
                .expect("Failed to finalize storage manager");
        }
        self.last_block =
            MockBlockHeader::from_height(self.last_block.height().checked_add(1).unwrap());
    }
}
