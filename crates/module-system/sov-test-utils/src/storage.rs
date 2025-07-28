use std::marker::PhantomData;
use std::path::Path;
use std::sync::{Arc, Mutex};

use rockbound::cache::delta_reader::DeltaReader;
use rockbound::SchemaBatch;
use sov_db::accessory_db::AccessoryDb;
use sov_db::config::RollupDbConfig;
use sov_db::historical_state::HistoricalStateReader;
use sov_db::ledger_db::LedgerDb;
use sov_db::state_db::StateDb;
use sov_db::state_db_nomt::get_session_builder_from_committed;
use sov_db::storage_manager::{InitializableNativeNomtStorage, InitializableNativeStorage};
pub use sov_db::storage_manager::{
    NativeChangeSet, NativeStorageManager, NomtChangeSet, NomtStorageManager,
};
use sov_mock_da::{MockBlockHeader, MockDaSpec};
use sov_modules_api::digest;
use sov_rollup_interface::da::{BlockHeaderTrait, DaSpec};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::nomt::prover_storage::NomtProverStorage;
use sov_state::{
    MerkleProofSpec, NativeStorage, ProverStorage, StateAccesses, Storage, StorageRoot,
};
use tempfile::TempDir;

use crate::TestSlotHash;

/// Implementation of [`HierarchicalStorageManager`] that provides [`ProverStorage`]
/// and commits changes directly to the underlying database.
pub struct SimpleStorageManager<S: MerkleProofSpec> {
    state: Arc<rockbound::DB>,
    accessory: Arc<rockbound::DB>,
    phantom_mp_spec: PhantomData<S>,
    // Holds ownership of [`Tempdir`] so it is not removed prematurely
    _dir: TempDir,
    root: StorageRoot<S>,
}

impl<S: MerkleProofSpec> Default for SimpleStorageManager<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: MerkleProofSpec> SimpleStorageManager<S> {
    /// Initialize new instance in temporary folder.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let state_rocksdb = StateDb::get_rockbound_options()
            .default_setup_db_in_path(dir.path())
            .unwrap();
        let accessory_rocksdb = AccessoryDb::get_rockbound_options()
            .default_setup_db_in_path(dir.path())
            .unwrap();
        Self {
            state: Arc::new(state_rocksdb),
            accessory: Arc::new(accessory_rocksdb),
            phantom_mp_spec: Default::default(),
            _dir: dir,
            root: <ProverStorage<S> as Storage>::PRE_GENESIS_ROOT,
        }
    }

    /// Do JMT genesis;
    pub fn genesis(&mut self) {
        if self.root != <ProverStorage<S> as Storage>::PRE_GENESIS_ROOT {
            panic!("Cannot call genesis on non empty storage");
        }
        let prover_storage = self.create_storage();
        let witness = S::Witness::default();
        let state_accesses_genesis = StateAccesses {
            user: Default::default(),
            kernel: Default::default(),
        };

        let (root, change_set) = prover_storage
            .compute_state_update(
                state_accesses_genesis,
                &witness,
                <ProverStorage<S> as Storage>::PRE_GENESIS_ROOT,
            )
            .expect("state update computation must succeed");

        let changes = prover_storage.materialize_changes(change_set);
        self.commit(changes);
        self.root = root;
    }

    /// Create a new [` ProverStorage `] that has a view only on data written to disc.
    pub fn create_storage(&self) -> ProverStorage<S> {
        let state_reader = DeltaReader::new(self.state.clone(), Vec::new());
        let state_db = StateDb::with_delta_reader(state_reader).unwrap();

        let accessory_reader = DeltaReader::new(self.accessory.clone(), Vec::new());
        let accessory_db = AccessoryDb::with_reader(accessory_reader).unwrap();
        ProverStorage::with_db_handles(state_db, accessory_db)
    }

    /// Saves changes directly to disk.
    // If we want it faster, can keep in memory
    pub fn commit(&mut self, stf_change_set: NativeChangeSet) {
        let NativeChangeSet {
            state_change_set,
            accessory_change_set,
        } = stf_change_set;
        self.state.write_schemas(&state_change_set).unwrap();
        self.accessory.write_schemas(&accessory_change_set).unwrap();
    }
}

/// Storage manager suitable for [`LedgerDb`].
pub struct SimpleLedgerStorageManager {
    db: Arc<rockbound::DB>,
}

impl SimpleLedgerStorageManager {
    /// Initialize a new instance in the given path.
    pub fn new(path: impl AsRef<std::path::Path>) -> Self {
        let db = LedgerDb::get_rockbound_options()
            .default_setup_db_in_path(path.as_ref())
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
    pub fn commit(&mut self, ledger_change_set: SchemaBatch) {
        self.db.write_schemas(&ledger_change_set).unwrap();
    }
}

/// Implementation of [`HierarchicalStorageManager`] that provides [`NomtProverStorage`]
/// and commits changes directly to the underlying database.
pub struct SimpleNomtStorageManager<S: MerkleProofSpec> {
    // Holds ownership of [`Tempdir`] so it is not removed prematurely
    _dir: TempDir,
    state: Arc<sov_db::state_db_nomt::NomtStateDb<S::Hasher>>,
    historical_state: Arc<rockbound::DB>,
    accessory: Arc<rockbound::DB>,
    root: StorageRoot<S>,
    is_strict_mode: bool,
}

impl<S: MerkleProofSpec> SimpleNomtStorageManager<S> {
    /// Initialize a new instance of [`SimpleNomtStorageManager`] in a temporary directory.
    pub fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = RollupDbConfig::default_in_path(dir.path().to_path_buf());
        let state_db = sov_db::state_db_nomt::NomtStateDb::new(config)
            .expect("Failed to initialize StateDb for NOMT");
        let historical_state_rocksdb = HistoricalStateReader::get_rockbound_options()
            .default_setup_db_in_path(dir.path())
            .unwrap();
        let accessory_rocksdb = AccessoryDb::get_rockbound_options()
            .default_setup_db_in_path(dir.path())
            .unwrap();

        Self {
            _dir: dir,
            state: Arc::new(state_db),
            historical_state: Arc::new(historical_state_rocksdb),
            accessory: Arc::new(accessory_rocksdb),
            root: <NomtProverStorage<S, TestSlotHash> as Storage>::PRE_GENESIS_ROOT,
            is_strict_mode: true,
        }
    }

    /// Change in which mode storage is going to be created.
    pub fn set_strict_mode(&mut self, use_strict_mode: bool) {
        self.is_strict_mode = use_strict_mode;
    }

    /// Create a new [`NomtProverStorage`] that has a view only on data written to disc.
    pub fn create_storage(&self) -> NomtProverStorage<S, TestSlotHash> {
        let state_session_builder = get_session_builder_from_committed(self.state.clone());
        let historical_state_reader = HistoricalStateReader::with_delta_reader(DeltaReader::new(
            self.historical_state.clone(),
            Vec::new(),
        ))
        .expect("Failed to create historical state reader");
        let accessory_db =
            AccessoryDb::with_reader(DeltaReader::new(self.accessory.clone(), Vec::new()))
                .expect("Failed to create accessory db");

        NomtProverStorage::create(
            state_session_builder,
            historical_state_reader,
            accessory_db,
            self.is_strict_mode,
        )
    }

    /// Commit [`NomtChangeSet`] to disk.
    pub fn commit(&mut self, stf_change_set: NomtChangeSet) {
        tracing::trace!("Committing changes to disk");
        let NomtChangeSet {
            state,
            historical_state,
            accessory,
        } = stf_change_set;

        self.state.commit_change_set(state).unwrap();
        tracing::trace!("Committed state changes to disk");
        self.accessory.write_schemas(&accessory).unwrap();
        tracing::trace!("Committed accessory changes to disk");
        self.historical_state
            .write_schemas(&historical_state)
            .unwrap();
        tracing::trace!("Committed historical state changes to disk");
        tracing::trace!("Committed all changes to disk");
    }
}

impl<S: MerkleProofSpec> Default for SimpleNomtStorageManager<S> {
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
    type Storage = ProverStorage<S>;

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

impl<S: MerkleProofSpec> ForklessStorageManager for SimpleNomtStorageManager<S> {
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

impl<Da: DaSpec, S: InitializableNativeStorage> PathInitializer for NativeStorageManager<Da, S> {
    fn new_in_path(path: impl AsRef<Path>) -> Self {
        Self::new(path.as_ref()).unwrap()
    }
}

impl<Da, H, S> PathInitializer for NomtStorageManager<Da, H, S>
where
    Da: DaSpec,
    H: digest::Digest<OutputSize = digest::typenum::U32> + Send + Sync,
    S: InitializableNativeNomtStorage<H, Da::SlotHash>,
{
    fn new_in_path(path: impl AsRef<Path>) -> Self {
        let config = RollupDbConfig::default_in_path(path.as_ref().to_path_buf());
        Self::new(config).unwrap()
    }
}

/// Using [`HierarchicalStorageManager`] to mimic [`SimpleStorageManager`],
/// but instead of commiting all data on disk, it just appends it to the following block.
/// Emulates fork-less DA without finality.
pub struct NonCommitingStorageManager<
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: Storage,
> {
    _dir: TempDir,
    // It holds mutex over the inner storage manager, for compatibility with the testing framework.
    storage_manager: Mutex<H>,
    last_block: MockBlockHeader,
    root: S::Root,
}

impl<H, S> NonCommitingStorageManager<H, S>
where
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: NativeStorage,
{
    /// Create the new [`NonCommitingStorageManager`].
    /// Passing [`TempDir`] allows keeping the directory from deletion.
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

impl<H, S> Default for NonCommitingStorageManager<H, S>
where
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: NativeStorage,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<H, S> ForklessStorageManager for NonCommitingStorageManager<H, S>
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
        // Here is the trick, we don't commit, but chain it to the last block
        self.root = new_root;
        self.storage_manager
            .lock()
            .unwrap()
            .save_change_set(&self.last_block, change_set, Default::default())
            .expect("Failed to save change set");
        self.last_block =
            MockBlockHeader::from_height(self.last_block.height().checked_add(1).unwrap());
    }
}

/// Storage manager that encapsulates MockDa with instant finality
pub struct CommitingStorageManager<
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: Storage,
> {
    _dir: TempDir,
    // It holds mutex over the inner storage manager, for compatibility with the testing framework.
    storage_manager: Mutex<H>,
    last_block: MockBlockHeader,
    root: S::Root,
}

impl<H, S> CommitingStorageManager<H, S>
where
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: NativeStorage,
{
    /// Create the new [`CommitingStorageManager`].
    /// Passing [`TempDir`] allows keeping the directory from deletion.
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

impl<H, S> Default for CommitingStorageManager<H, S>
where
    H: HierarchicalStorageManager<MockDaSpec, StfState = S> + PathInitializer,
    S: NativeStorage,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<H, S> ForklessStorageManager for CommitingStorageManager<H, S>
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
        // Here is the trick, we don't commit, but chain it to the last block
        self.root = new_root;
        let mut storage_manager = self.storage_manager.lock().unwrap();
        storage_manager
            .save_change_set(&self.last_block, change_set, Default::default())
            .expect("Failed to save change set");
        storage_manager
            .finalize(&self.last_block)
            .expect("Failed to finalize storage manager");
        self.last_block =
            MockBlockHeader::from_height(self.last_block.height().checked_add(1).unwrap());
    }
}
