//! Prover side of NOMT-based Storage implementation
use std::any::Any;
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt::Formatter;
use std::io::Write;
use std::time::{Duration, Instant};

use anyhow::Context;
use nomt::hasher::BinaryHasher;
use nomt::proof::MultiProof;
use nomt::FinishedSession;
use nomt_core::trie::KeyPath;
use sov_db::accessory_db::AccessoryDb;
use sov_db::historical_state::HistoricalStateReader;
use sov_db::state_db_nomt::{HistoricalValueError, NomtSessionBuilder, SessionsContainer};
use sov_db::storage_manager::{
    InitializableNativeNomtStorage, NomtChangeSet, StateFinishedSession, WitnessMode,
};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::reexports::digest::Digest;

use crate::nomt::NomtMultiProof;
use crate::pinned_cache::PinnedCache;
use crate::storage::ReadType;
use crate::{
    Accessory, CompileTimeNamespace, Kernel, MerkleProofSpec, Namespace, NativeStorage, NodeLeaf,
    NodeLeafAndMaybeValue, OrderedReadsAndWrites, ProvableCompileTimeNamespace, ProvableNamespace,
    SlotKey, SlotValue, StateAccesses, StateRoot, StateUpdate, Storage, StorageProof, StorageRoot,
    User, Witness,
};

type NomtSession<H> = nomt::Session<BinaryHasher<H>>;

#[derive(Debug)]
enum GetWithProofError {
    StateRootMismatch,
    Other(anyhow::Error),
}

impl core::fmt::Display for GetWithProofError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StateRootMismatch => {
                write!(f, "State root mismatch between pre-fetch and post-fetch")
            }
            Self::Other(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for GetWithProofError {}

impl From<anyhow::Error> for GetWithProofError {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err)
    }
}

/// A [`Storage`] implementation to be used by the prover in a native execution based on NOMT.
pub struct NomtProverStorage<S: MerkleProofSpec, K>
where
    K: Clone,
{
    state_session_builder: NomtSessionBuilder<S::Hasher, K>,
    historical_state: HistoricalStateReader,
    accessory: AccessoryDb,
    /// Enable staleness/consistency checks (NOMT vs rocksdb, root hash comparison).
    /// Should be true for all node-context block processing.
    strict_mode: bool,
    /// Controls witness hint generation for ZK proofs and optional pinned cache.
    witness_mode: WitnessMode<PinnedCache>,
}

impl<S: MerkleProofSpec, K: Clone> Clone for NomtProverStorage<S, K>
where
    S: MerkleProofSpec,
    K: Clone,
{
    fn clone(&self) -> Self {
        if matches!(
            self.witness_mode,
            WitnessMode::Off {
                pinned_cache: Some(_)
            }
        ) {
            tracing::warn!("Cloning NomtProverStorage which has an active pinned cache. The pinned cache will not be propagated to the clone.");
        }
        let witness_mode = if self.witness_mode.is_witness_enabled() {
            WitnessMode::On
        } else {
            WitnessMode::off()
        };
        Self {
            state_session_builder: self.state_session_builder.clone(),
            historical_state: self.historical_state.clone(),
            accessory: self.accessory.clone(),
            strict_mode: self.strict_mode,
            witness_mode,
        }
    }
}

impl<S: MerkleProofSpec, K> core::fmt::Debug for NomtProverStorage<S, K>
where
    K: Clone,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "NomtProverStorage::<{}>", std::any::type_name::<S>())
    }
}

impl<S: MerkleProofSpec, K> NomtProverStorage<S, K>
where
    K: Clone,
{
    /// Create the new instance of [`NomtProverStorage`] with the given sessions.
    /// If `strict_mode` is true, consistency checks between NOMT and rocksdb will be performed.
    /// Please check [`NomtProverStorage::should_check_dbs_sync`] for more details.
    pub fn create(
        state_session_builder: NomtSessionBuilder<S::Hasher, K>,
        historical_state: HistoricalStateReader,
        accessory: AccessoryDb,
        strict_mode: bool,
        witness_mode: WitnessMode<PinnedCache>,
    ) -> Self {
        Self {
            state_session_builder,
            historical_state,
            accessory,
            strict_mode,
            witness_mode,
        }
    }
    /// Utility method for checking if storage is empty.
    /// Does not guarantee 100% that it actually is.
    pub fn is_empty(&self) -> bool {
        self.historical_state.get_next_version() == SlotNumber::GENESIS
    }

    /// Allows changing strict mode for the existing storage.
    #[cfg(feature = "test-utils")]
    pub fn change_strict_mode(&mut self, use_strict_mode: bool) {
        self.strict_mode = use_strict_mode;
    }

    /// Returns a double option: The outer option is `None` if there is no reasonable version to use,
    /// while the inner option is None if we want to get the latest state rather than querying for a
    /// particular version.
    fn get_version_to_use(&self, version: Option<SlotNumber>) -> Option<Option<SlotNumber>> {
        if self.is_empty() {
            return None;
        }
        let next_version = self.historical_state.get_next_version();
        match version {
            None => Some(None),
            Some(passed_version) => {
                if passed_version >= next_version {
                    None
                } else {
                    Some(Some(passed_version))
                }
            }
        }
    }
}

impl<S: MerkleProofSpec, K> NomtProverStorage<S, K>
where
    K: Clone + Eq + std::hash::Hash,
{
    /// Indicates if data consistency check between NOMT and rocksdb should be performed.
    /// The check only happens if `strict_mode` is enabled, the storage is past the genesis version and
    /// the version to use is the latest known to this storage.
    /// Strict mode implies that `latest_version()` is the **total latest** version,
    /// not the latest known to this storage.
    fn should_check_dbs_sync(&self, version_to_use: SlotNumber) -> bool {
        cfg!(debug_assertions) &&
            self.strict_mode
            // latest version can be equal to genesis in 2 cases: pre-genesis and at genesis.
            // Since genesis is a special case and not covered by normal stf transition,
            // we exclude this case for simpler testing.
            && version_to_use > SlotNumber::GENESIS
            && version_to_use == self.latest_version()
    }

    /// Reads the latest value for the given key without trying to preserve the illusion that this `Storage` instance is backed by a snapshot.
    /// If the underlying DB has advanced beyond the snapshot, this method will read from the live DB.
    fn read_value_unbound<N: CompileTimeNamespace>(&self, key: &SlotKey) -> Option<SlotValue> {
        match N::NAMESPACE {
            Namespace::User => self
                .historical_state
                .get_user_value_option_by_key_unbound(key)
                .expect("Unable to read from UserDb"),
            Namespace::Kernel => self
                .historical_state
                .get_kernel_value_option_by_key_unbound(key)
                .expect("Unable to read from KernelDb"),
            Namespace::Accessory => self
                .accessory
                .get_value_option(key, SlotNumber::MAX)
                .expect("Unable to read from AccessoryDb")
                .map(Into::into),
        }
    }

    fn read_value<N: CompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
    ) -> Result<Option<SlotValue>, HistoricalValueError> {
        // Note that the resolved version is logged but *not* necessarily passed to the backing DB.
        // Passing a version indicates intent to perform a historical (rather than live) query, which
        // bypasses the cache. This causes worse read performance, but prevents queries from interfering with
        // transaction execution by thrashing the cache.
        let Some(resolved_version) = self.get_version_to_use(version) else {
            return Ok(None);
        };
        let _span = tracing::debug_span!("NomtProverStorage::read_value", ?resolved_version, passed_version = ?version).entered();
        let val = match N::NAMESPACE {
            Namespace::User => {
                let historical_value = if let Some(version) = resolved_version {
                    self.historical_state
                        .get_user_value_option_by_key_historical(key, version)?
                } else {
                    self.historical_state.get_user_value_option_by_key(key)?
                };
                let version_to_check = resolved_version.unwrap_or(self.latest_version());
                if self.should_check_dbs_sync(version_to_check) {
                    let key_path = S::Hasher::digest(key.as_ref()).into();

                    let nomt_session = self
                        .state_session_builder
                        .begin_user_session_without_witness()
                        .expect("Failed to build user session");
                    let nomt_value = nomt_session.read(key_path).unwrap();
                    drop(nomt_session);
                    let historical_value_hash = historical_value
                        .as_ref()
                        .map(|v| v.combine_val_hash_and_size::<S::Hasher>());
                    assert_eq!(nomt_value, historical_value_hash);
                }

                historical_value
            }
            Namespace::Kernel => {
                let historical_value = if let Some(version) = version {
                    self.historical_state
                        .get_kernel_value_option_by_key_historical(key, version)?
                } else {
                    self.historical_state.get_kernel_value_option_by_key(key)?
                };
                let version_to_check = resolved_version.unwrap_or(self.latest_version());
                if self.should_check_dbs_sync(version_to_check) {
                    let key_path = S::Hasher::digest(key.as_ref()).into();
                    let nomt_session = self
                        .state_session_builder
                        .begin_kernel_session_without_witness()
                        .expect("Failed to build kernel session");
                    let nomt_value = nomt_session.read(key_path).unwrap();
                    drop(nomt_session);
                    let historical_value_hash = historical_value
                        .as_ref()
                        .map(|v| v.combine_val_hash_and_size::<S::Hasher>());
                    assert_eq!(nomt_value, historical_value_hash);
                }

                historical_value
            }
            Namespace::Accessory => self
                .accessory
                .get_value_option(key, resolved_version.unwrap_or(self.latest_version()))
                .expect("Unable to read from AccessoryDb")
                .map(Into::into),
        };
        Ok(val)
    }

    fn do_get_leaf<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        witness: Option<&<Self as Storage>::Witness>,
    ) -> Result<Option<NodeLeafAndMaybeValue>, HistoricalValueError> {
        let val = self.read_value::<N>(key, version)?;

        // First, we create a node that we put in the cache. This one contains the value.
        let node_leaf_with_fetched_value = val.map(|v| {
            let leaf = NodeLeaf::make_leaf::<S::Hasher>(&v);
            NodeLeafAndMaybeValue {
                leaf,
                value: ReadType::GetSizeValueFetched(v),
            }
        });

        // Second, we create a node that we put in the witness. This one doesn't contain the value.
        let node_leaf_without_value =
            node_leaf_with_fetched_value
                .clone()
                .map(|node| NodeLeafAndMaybeValue {
                    leaf: node.leaf,
                    value: ReadType::GetSizeValueNotFetched,
                });

        if let Some(witness) = witness {
            witness.add_hint(&node_leaf_without_value);
        }
        Ok(node_leaf_with_fetched_value)
    }

    fn materialize_changes_with_version(
        self,
        state_update: NomtStateUpdate<S>,
        version: SlotNumber,
    ) -> NomtChangeSet {
        tracing::trace!(
            %version,
            "NomtProverStorage, materializing changes at explicit version"
        );
        let NomtStateUpdate {
            state_accesses:
                StateAccesses {
                    user: user_versioned,
                    kernel: kernel_versioned,
                },
            accessory: accessory_writes,
            user,
            kernel,
            next_root_hash,
            pinned_cache,
        } = state_update;
        let user_to_materialize = user_versioned.ordered_writes.into_iter();
        let kernel_to_materialize = kernel_versioned.ordered_writes.into_iter();
        let historical_schema_batch = HistoricalStateReader::materialize_values(
            user_to_materialize,
            kernel_to_materialize,
            borsh::to_vec(&next_root_hash).expect("Failed to serialize root hash"),
            version,
        )
        .expect("historical state db materialization must succeed");
        let accessory_batch = AccessoryDb::materialize_values(
            accessory_writes
                .ordered_writes
                .iter()
                // TODO(@preston-evans98) Skip the useless to_vec here. https://github.com/Sovereign-Labs/sovereign-sdk/issues/1824
                .map(|(k, v_opt)| {
                    (
                        k.as_ref().to_vec(),
                        v_opt.as_ref().map(|v| v.value().to_vec()),
                    )
                }),
            version,
        )
        .expect("accessory db materialization must succeed");
        // Erase the type of the pinned cache since the storage manager isn't aware of it.
        let pinned_cache = pinned_cache.map(|c| Box::new(c) as Box<dyn Any + Send + Sync>);
        NomtChangeSet {
            state: StateFinishedSession::new(user, kernel),
            historical_state: historical_schema_batch,
            accessory: accessory_batch,
            pinned_cache,
        }
    }

    /// Materializes a state update at an explicit version.
    ///
    /// This is intended for offline migration tooling that updates state in-place at the
    /// current head version instead of appending a new version.
    pub fn materialize_changes_at_version(
        self,
        state_update: NomtStateUpdate<S>,
        version: SlotNumber,
    ) -> NomtChangeSet {
        self.materialize_changes_with_version(state_update, version)
    }

    /// Get the latest root hash available in the live db. This could be the newest root hash from the underlying db, or the root hash from the latest delta in memory - whichever is newer.
    fn latest_root_and_version_unbound(&self) -> Option<(SlotNumber, StorageRoot<S>)> {
        self.historical_state
            .latest_root_and_version_unbound()
            .expect("Error reading from database")
            .map(|(version, root)| {
                (
                    version,
                    borsh::from_slice(&root).expect("Error deserializing root hash"),
                )
            })
    }

    // Fetch the requested key with proof. Atomically retrieve the values of any provided accessory keys at the same storage version.
    //
    // This is a single attempt at the latest-state algorithm. The public `get_with_proof`
    // wrapper retries only `StateRootMismatch`, which is the transient case caused by NOMT and
    // RocksDB committing slightly out of phase. All other failures are returned immediately.
    //
    // NOMT only maintains the live version of the merkle tree
    //
    // We want to ensure that the merkle proof we fetch (from NOMT) and the value (from RocksDB) are consistent. But RocksDB and NOMT commit at slightly different times.
    // So, what we do is...
    // 1. Fetch the root hash from RocksDB at the latest version.
    // 2. Read the value from RocksDB
    // 3. Fetch the root hash from RocksDB again.
    // 4. Open a NOMT session (which locks NOMT and prevents any commits)
    // 5. Compare NOMT's state root with the ones we fetched from RocksDB. If they don't match, GOTO 1.
    // 6. Generate the merkle proof
    // 7. (Implicit) unlock NOMT by dropping the session.
    //
    // This algorithm guarantees that NOMT and RocksDB are consistent, since the root hash at the time of the RocksDB read is known (we checked both before and after the read) and we've
    // compared that it matches the NOMT root.
    //
    // Note that in this function, we always the use the "unbound" methods to read from storage. Recall that the standard readers attempt to preserve the illusion that the storage holds a point-in-time snapshot,
    // so they fall back to archival state if the underlying DB has changed. But NOMT doesn't have an archival tree to make merkle proofs against, so this behavior would break consistency.
    // The "unbound" methods bypass this behavior and always show the newest available value (either a value from the in-memory deltas in the storage, or - if it's newer - the value from the underlying database).
    //
    // Aside: Why do we need to fetch the root hash twice?
    // If we only fetch once, there's a subtle race condition. With a single read, one or the other of these interleavings is possible: (Read_rocksdb_value-> (other thread)commit_rocksdb -> Read_rocksdb_root) or (Read_rocksdb_root-> (other thread)commit_rocksdb -> Read_rocksdb_value)
    // In either case, the value we read from RocksDB will be inconsistent with the root hash we fetched from RocksDB. By reading the root hash before and after the value and cross-checking, we eliminate this possibility.
    fn get_with_proof_once<N: ProvableCompileTimeNamespace>(
        &self,
        proven_key: SlotKey,
    ) -> Result<(StorageProof<NomtMultiProof>, SlotNumber, StorageRoot<S>), GetWithProofError> {
        let namespace = N::PROVABLE_NAMESPACE;
        // Fetch the latest root hash from the newest delta or the live table, whichever is newer.
        let (_, pre_fetch_state_root) =
            self.latest_root_and_version_unbound().ok_or_else(|| {
                GetWithProofError::Other(anyhow::anyhow!("Latest root hash not found"))
            })?;
        let pre_fetch_state_root = pre_fetch_state_root.namespace_root(namespace);
        // Read the value from the newest delta or the live table, whichever is newer.
        let value = match namespace {
            ProvableNamespace::User => self.read_value_unbound::<User>(&proven_key),
            ProvableNamespace::Kernel => self.read_value_unbound::<Kernel>(&proven_key),
        };
        // Fetch the latest root hash again. As before, uses the newest delta or the live table, whichever is newer.
        let (committed_slot_number, post_fetch_state_root) = self
            .latest_root_and_version_unbound()
            .ok_or(anyhow::anyhow!("Latest root hash not found"))?;
        let post_fetch_state_root_namespace = post_fetch_state_root.namespace_root(namespace);

        let key_path: KeyPath = S::Hasher::digest(proven_key.as_ref()).into();
        let session = match namespace {
            ProvableNamespace::User => self
                .state_session_builder
                .begin_user_session_without_witness()
                .map_err(GetWithProofError::from)?,
            ProvableNamespace::Kernel => self
                .state_session_builder
                .begin_kernel_session_without_witness()
                .map_err(GetWithProofError::from)?,
        };

        if pre_fetch_state_root != post_fetch_state_root_namespace
            || post_fetch_state_root_namespace != session.prev_root().as_ref()
        {
            return Err(GetWithProofError::StateRootMismatch);
        }

        let path_proof = session.prove(key_path).map_err(GetWithProofError::from)?;
        drop(session);
        let multi_proof = MultiProof::from_path_proofs(vec![path_proof]);

        Ok((
            StorageProof {
                key: proven_key,
                value,
                proof: NomtMultiProof(multi_proof),
                namespace,
            },
            committed_slot_number,
            post_fetch_state_root,
        ))
    }
}

fn to_nomt_accesses<S: MerkleProofSpec>(
    sov_accesses: &OrderedReadsAndWrites,
) -> anyhow::Result<Vec<(nomt::trie::KeyPath, nomt::KeyReadWrite)>> {
    let mut merged_accesses: BTreeMap<nomt::trie::KeyPath, nomt::KeyReadWrite> = BTreeMap::new();

    let OrderedReadsAndWrites {
        ordered_reads,
        ordered_writes,
    } = sov_accesses;

    // First, put all the reads into merged accesses, so later we can distinguish `Write` from `ReadThenWrite`
    for (key, read_node_leaf) in ordered_reads {
        // Reads are warmed up during normal `get/get_leaf`
        let key_hash: nomt::trie::KeyPath = S::Hasher::digest(key.as_ref()).into();

        let combined_hash_and_size =
            read_node_leaf.map(|node_leaf| node_leaf.combine_val_hash_and_size());

        let nomt_read = nomt::KeyReadWrite::Read(combined_hash_and_size);

        if merged_accesses.insert(key_hash, nomt_read).is_some() {
            anyhow::bail!("Duplicate key read in state: {:?}", key_hash);
        };
    }

    // Writes
    for (key, original_write) in ordered_writes {
        let key_hash: nomt::trie::KeyPath = S::Hasher::digest(key.as_ref()).into();

        let authenticated_write = original_write
            .as_ref()
            .map(|v| v.combine_val_hash_and_size::<S::Hasher>());

        match merged_accesses.entry(key_hash) {
            Entry::Vacant(vacant) => {
                // Also warming up all writes. `ReadThenWrite` has been warmed up during reads collection.
                vacant.insert(nomt::KeyReadWrite::Write(authenticated_write));
            }
            Entry::Occupied(occupied) => match occupied.remove() {
                nomt::KeyReadWrite::Read(read_value) => {
                    merged_accesses.insert(
                        key_hash,
                        nomt::KeyReadWrite::ReadThenWrite(read_value, authenticated_write),
                    );
                }
                _ => {
                    anyhow::bail!("Duplicate key write in state: {:?}", key_hash);
                }
            },
        }
    }

    Ok(merged_accesses.into_iter().collect())
}

fn compute_state_update_namespace<S: MerkleProofSpec>(
    session: NomtSession<S::Hasher>,
    accesses: Vec<(nomt::trie::KeyPath, nomt::KeyReadWrite)>,
    witness: &S::Witness,
    write_witness: bool,
) -> anyhow::Result<FinishedSession> {
    tracing::trace!(accesses = accesses.len(), "compute state update");
    let mut finished = session.finish(accesses)?;
    if write_witness {
        let nomt_witness = finished.take_witness().expect("Witness cannot be missing");
        let nomt::Witness {
            path_proofs,
            operations: nomt::WitnessedOperations { .. },
        } = nomt_witness;
        // Note, we discard `p.path`, but maybe there's a way to use to have more efficient verification?
        let mut path_proofs_inner = path_proofs.into_iter().map(|p| p.inner).collect::<Vec<_>>();

        // Sort them as required by
        // Note that the path proofs produced within a crate::witness::Witness are not guaranteed to be ordered,
        // so the input should be sorted lexicographically by the terminal path prior to calling this function.
        // https://github.com/thrumdev/nomt/issues/904
        path_proofs_inner.sort_by(|a, b| a.terminal.path().cmp(b.terminal.path()));

        let multi_proof = MultiProof::from_path_proofs(path_proofs_inner);
        witness.add_hint(&multi_proof);
    }
    Ok(finished)
}

impl<S: MerkleProofSpec, K> InitializableNativeNomtStorage<S::Hasher, K> for NomtProverStorage<S, K>
where
    K: Clone + Send + Sync,
{
    fn new(
        state_db: NomtSessionBuilder<S::Hasher, K>,
        historical_state: HistoricalStateReader,
        accessory_db: AccessoryDb,
        strict_mode: bool,
        witness_mode: WitnessMode,
    ) -> Self {
        let witness_mode = match witness_mode {
            WitnessMode::On => WitnessMode::On,
            WitnessMode::Off { pinned_cache } => {
                let pinned_cache: Option<PinnedCache> = pinned_cache.map(|c| *c.downcast().expect("Failed to downcast the pinned_cache argument to `NomtProverStorage`. This is a bug. Please report it."));
                WitnessMode::Off { pinned_cache }
            }
        };
        Self::create(
            state_db,
            historical_state,
            accessory_db,
            strict_mode,
            witness_mode,
        )
    }
}

#[allow(missing_docs)]
pub struct NomtStateUpdate<S: MerkleProofSpec> {
    user: FinishedSession,
    kernel: FinishedSession,
    accessory: OrderedReadsAndWrites,
    state_accesses: StateAccesses,
    next_root_hash: StorageRoot<S>,
    pinned_cache: Option<PinnedCache>,
}

impl<S: MerkleProofSpec> StateUpdate for NomtStateUpdate<S> {
    fn add_accessory_item(&mut self, key: SlotKey, value: Option<SlotValue>) {
        self.accessory.ordered_writes.push((key, value));
    }

    fn get_accessory_items(&self) -> impl Iterator<Item = &(SlotKey, Option<SlotValue>)> {
        self.accessory.ordered_writes.iter()
    }
}

impl<S: MerkleProofSpec, K> Storage for NomtProverStorage<S, K>
where
    K: Clone + Eq + std::hash::Hash,
{
    type Hasher = S::Hasher;
    type Witness = S::Witness;
    type Proof = NomtMultiProof;
    type Root = StorageRoot<S>;
    // These 2 are effectively the same thing, `StateUpdate` is not materialized, `ChangeSet` is materialized.
    type StateUpdate = NomtStateUpdate<S>;
    type ChangeSet = NomtChangeSet;
    const PRE_GENESIS_ROOT: Self::Root =
        StorageRoot::new(nomt::trie::TERMINATOR, nomt::trie::TERMINATOR);

    fn put_in_witness(&self, value: Option<SlotValue>, witness: &Self::Witness) {
        if self.witness_mode.is_witness_enabled() {
            witness.add_hint(&value);
        }
    }

    fn get_leaf<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        let witness_ref = if self.witness_mode.is_witness_enabled() {
            Some(witness)
        } else {
            None
        };
        match self.do_get_leaf::<N>(key, None, witness_ref) {
            Ok(val) => val,
            Err(e) => {
                // Historical errors are not expected when fetching without a version
                panic!("Database error while getting leaf: for key {key}. error: {e:?}");
            }
        }
    }

    fn get<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<SlotValue> {
        match self.read_value::<N>(key, None) {
            Ok(val) => {
                if self.witness_mode.is_witness_enabled() {
                    witness.add_hint(&val);
                }
                val
            }
            Err(e) => {
                // Historical errors are not allowed when fetching without a version
                panic!("Database error while getting value for key {key}. error: {e:?}");
            }
        }
    }

    fn get_accessory(&self, key: &SlotKey) -> Option<SlotValue> {
        match self.read_value::<Accessory>(key, None) {
            Ok(val) => val,
            Err(e) => {
                // Historical errors are not allowed when fetching without a version
                panic!("Database error while getting value for accessory key {key}. error: {e:?}");
            }
        }
    }

    fn compute_state_update(
        &self,
        state_accesses: StateAccesses,
        witness: &Self::Witness,
        prev_state_root: Self::Root,
        pinned_cache: Option<PinnedCache>,
    ) -> anyhow::Result<(Self::Root, Self::StateUpdate)> {
        let start = std::time::Instant::now();
        let nomt_accesses_user = to_nomt_accesses::<S>(&state_accesses.user)?;
        let nomt_accesses_kernel = to_nomt_accesses::<S>(&state_accesses.kernel)?;
        let accesses_build_time = start.elapsed();
        tracing::trace!(time = ?accesses_build_time, "Nomt accesses are computed");

        let next_version = self.historical_state.get_next_version();
        let start_session = std::time::Instant::now();
        // Open 2 sessions at the same time
        let SessionsContainer {
            user: user_session,
            kernel: kernel_session,
        } = self
            .state_session_builder
            .begin_both_sessions(self.witness_mode.is_witness_enabled())?;
        let starting_session_time = start_session.elapsed();
        tracing::debug!(%prev_state_root, %next_version, sesssion_starting_time = ?starting_session_time, "computing state update, sessions are live");

        let current_prev_user_root = user_session.prev_root().into_inner();
        let current_prev_kernel_root = kernel_session.prev_root().into_inner();
        let current_prev_root = StorageRoot::new(current_prev_user_root, current_prev_kernel_root);

        // Check staleness, pre-computation:
        if self.strict_mode && current_prev_root != prev_state_root {
            anyhow::bail!("stale storage on next_version={}, passed prev_state_root {} does not match the current prev_state_root {}",
                next_version,
                prev_state_root,
                current_prev_root
            );
        }

        let sessions_start = std::time::Instant::now();
        let user_finished_session = {
            let _span = tracing::debug_span!("compute_state_update", namespace = "user").entered();
            compute_state_update_namespace::<S>(
                user_session,
                nomt_accesses_user,
                witness,
                self.witness_mode.is_witness_enabled(),
            )
            .context("user state")?
        };
        let kernel_finished_session = {
            let _span =
                tracing::debug_span!("compute_state_update", namespace = "kernel").entered();
            compute_state_update_namespace::<S>(
                kernel_session,
                nomt_accesses_kernel,
                witness,
                self.witness_mode.is_witness_enabled(),
            )
            .context("kernel state")?
        };
        let finishing_session_time = sessions_start.elapsed();

        let user_reads = state_accesses.user.ordered_reads.len();
        let user_writes = state_accesses.user.ordered_writes.len();
        let kernel_reads = state_accesses.kernel.ordered_reads.len();
        let kernel_writes = state_accesses.kernel.ordered_writes.len();
        let with_witness = self.witness_mode.is_witness_enabled();
        sov_metrics::track_metrics(|tracker| {
            tracker.submit(NomtProverComputeStateResult {
                user_reads,
                user_writes,
                kernel_reads,
                kernel_writes,
                with_witness,
            });
        });

        // Additional self-check that the finished session has the same previous root hash as passed prev_state_root.
        let kernel_finished_session_prev_root = kernel_finished_session.prev_root().into_inner();
        let user_finished_session_prev_root = user_finished_session.prev_root().into_inner();
        let finished_session_prev_root = StorageRoot::new(
            user_finished_session_prev_root,
            kernel_finished_session_prev_root,
        );

        // Check staleness, post-computation. This should check if storage became stale during the computation.
        if self.strict_mode && prev_state_root != finished_session_prev_root {
            anyhow::bail!("stale storage on next_version={}, passed prev_state_root {} does not match the current prev_state_root {}",
                next_version,
                prev_state_root,
                current_prev_root
            );
        }

        let user_root = user_finished_session.root();
        let kernel_root = kernel_finished_session.root();
        let root = StorageRoot::new(user_root.into_inner(), kernel_root.into_inner());

        tracing::debug!(state_root = %root, %next_version, time = ?start.elapsed(), ?accesses_build_time, ?finishing_session_time, "computed next state root");

        let state_update = NomtStateUpdate {
            user: user_finished_session,
            kernel: kernel_finished_session,
            accessory: Default::default(),
            state_accesses,
            next_root_hash: root,
            pinned_cache,
        };

        Ok((root, state_update))
    }

    fn materialize_changes(self, state_update: Self::StateUpdate) -> Self::ChangeSet {
        let next_version = self.historical_state.get_next_version();
        self.materialize_changes_with_version(state_update, next_version)
    }

    fn open_proof(
        state_root: Self::Root,
        proof: StorageProof<Self::Proof>,
    ) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
        crate::nomt::verify_storage_proof::<S>(state_root, proof)
    }
}

impl<S: MerkleProofSpec, K> NativeStorage for NomtProverStorage<S, K>
where
    K: Clone + Eq + std::hash::Hash,
{
    fn latest_version(&self) -> SlotNumber {
        self.historical_state.get_next_version().saturating_sub(1)
    }

    fn latest_version_unbound(&self) -> SlotNumber {
        self.historical_state
            .last_version_unbound()
            .expect("Issue with underlying database")
    }

    fn get_historical<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> anyhow::Result<Option<SlotValue>> {
        Ok(self.read_value::<N>(key, version)?)
    }

    fn get_leaf_historical<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> anyhow::Result<Option<NodeLeafAndMaybeValue>> {
        Ok(self.do_get_leaf::<N>(key, version, None)?)
    }

    fn get_accessory_historical(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
    ) -> anyhow::Result<Option<SlotValue>> {
        Ok(self.read_value::<Accessory>(key, version)?)
    }

    fn get_with_proof<N: ProvableCompileTimeNamespace>(
        &self,
        proven_key: SlotKey,
    ) -> anyhow::Result<(StorageProof<Self::Proof>, SlotNumber, Self::Root)> {
        // Keep the retry budget well under the RPC's 500ms target while avoiding hammering the
        // DB under sustained write pressure. In practice one blocked attempt often lands after the
        // in-flight commit finishes, so a few short retries are enough for the transient mismatch.
        const RETRY_DEADLINE: Duration = Duration::from_millis(375);
        const RETRY_BACKOFFS_MS: [u64; 3] = [2, 10, 25];

        let start = Instant::now();
        let mut attempts = 0;

        loop {
            attempts += 1;
            match self.get_with_proof_once::<N>(proven_key.clone()) {
                Ok(result) => return Ok(result),
                Err(GetWithProofError::Other(err)) => return Err(err),
                Err(GetWithProofError::StateRootMismatch) => {
                    let Some(backoff_ms) = RETRY_BACKOFFS_MS.get(attempts - 1).copied() else {
                        anyhow::bail!(
                            "State root mismatch between pre-fetch and post-fetch after {attempts} attempts over {:?}",
                            start.elapsed()
                        );
                    };

                    let elapsed = start.elapsed();
                    if elapsed >= RETRY_DEADLINE {
                        anyhow::bail!(
                            "State root mismatch between pre-fetch and post-fetch after {attempts} attempts over {:?}",
                            elapsed
                        );
                    }

                    let remaining = RETRY_DEADLINE.saturating_sub(elapsed);
                    std::thread::sleep(Duration::from_millis(backoff_ms).min(remaining));
                }
            }
        }
    }

    fn get_root_hash(&self, version: SlotNumber) -> anyhow::Result<Self::Root> {
        let version_to_use = match self.get_version_to_use(Some(version)) {
            None => {
                // Mimic error from jmt, historical reasons.
                anyhow::bail!("Root node not found for version {}.", version)
            }
            Some(v) => v,
        }
        .unwrap_or(self.latest_version());
        let storage_root_historical = self.get_root_hash_unbound(version_to_use)?;
        if self.should_check_dbs_sync(version_to_use) {
            let session_container = self.state_session_builder.begin_both_sessions(false)?;
            let user_root = session_container.user.prev_root();
            let kernel_root = session_container.kernel.prev_root();
            drop(session_container);
            let prev_root_nomt = StorageRoot::new(user_root.into_inner(), kernel_root.into_inner());
            assert_eq!(
                storage_root_historical, prev_root_nomt,
                "Root hash mismatch between historical and nomt databases"
            );
        }

        Ok(storage_root_historical)
    }

    fn get_root_hash_unbound(&self, version: SlotNumber) -> anyhow::Result<Self::Root> {
        let raw_root = self
            .historical_state
            .get_serialized_root_hash(version)?
            .context(format!("Root hash not found for version {version}."))?;
        let storage_root_historical =
            borsh::from_slice(&raw_root).expect("Failed to deserialize root hash");
        tracing::trace!(%version, root_hash = %storage_root_historical, "Got unbound root hash");
        Ok(storage_root_historical)
    }

    fn get_unbound<N: CompileTimeNamespace>(&self, key: SlotKey) -> Option<SlotValue> {
        self.read_value_unbound::<N>(&key)
    }

    fn get_accessory_unbound(
        &self,
        key: SlotKey,
        max_version: Option<SlotNumber>,
    ) -> Option<SlotValue> {
        self.accessory
            .get_value_option(&key, max_version.unwrap_or(SlotNumber::MAX))
            .expect("Unable to read from AccessoryDb")
            .map(Into::into)
    }

    fn maybe_iter_user_values_with_prefix(
        &self,
        prefix: SlotKey,
    ) -> anyhow::Result<Option<impl Iterator<Item = (SlotKey, SlotValue)>>> {
        let iter = self
            .historical_state
            .iter_user_values_with_prefix(&prefix)?;
        let Some(iter) = iter else {
            return Ok(None);
        };

        Ok(Some(
            iter.filter_map(|(key, value)| value.map(|v| (key, v))),
        ))
    }

    fn maybe_iter_kernel_values_with_prefix(
        &self,
        prefix: SlotKey,
    ) -> anyhow::Result<Option<impl Iterator<Item = (SlotKey, SlotValue)>>> {
        let iter = self
            .historical_state
            .iter_kernel_values_with_prefix(&prefix)?;
        let Some(iter) = iter else {
            return Ok(None);
        };

        Ok(Some(
            iter.filter_map(|(key, value)| value.map(|v| (key, v))),
        ))
    }

    fn try_load_saved_pinned_cache(&mut self) -> Option<PinnedCache> {
        self.witness_mode.take_pinned_cache()
    }
}

/// Metric for number of reads and writes in both namespaces that have been passed to `compute_state_update`
#[derive(Clone, Debug)]
pub struct NomtProverComputeStateResult {
    user_reads: usize,
    user_writes: usize,
    kernel_reads: usize,
    kernel_writes: usize,
    with_witness: bool,
}

impl sov_metrics::Metric for NomtProverComputeStateResult {
    fn measurement_name(&self) -> &'static str {
        "sov_nomt_prover_compute_state"
    }

    fn serialize_for_telegraf(&self, buffer: &mut Vec<u8>) -> std::io::Result<()> {
        let name = self.measurement_name();
        let user_reads = self.user_reads;
        let user_writes = self.user_writes;
        let kernel_reads = self.kernel_reads;
        let kernel_writes = self.kernel_writes;
        let with_witness = self.with_witness as u8;
        write!(buffer, "{name},with_witness={with_witness} user_reads={user_reads},user_writes={user_writes},kernel_reads={kernel_reads},kernel_writes={kernel_writes}")
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sha2::Sha256;
    use sov_db::config::RollupDbConfig;
    use sov_db::storage_manager::NomtStorageManager;
    use sov_db::test_utils::CommitFaultInjectionLocation;
    use sov_mock_da::{MockBlockHeader, MockDaSpec, MockHash};
    use sov_rollup_interface::storage::HierarchicalStorageManager;

    use super::{GetWithProofError, NomtProverStorage};
    use crate::cache::{OrderedReadsAndWrites, StateAccesses};
    use crate::storage::{NativeStorage, StateUpdate, Storage};
    use crate::{DefaultStorageSpec, SlotKey, SlotValue, User};

    type TestStorage = NomtProverStorage<DefaultStorageSpec<Sha256>, MockHash>;
    type TestStorageManager = NomtStorageManager<MockDaSpec, Sha256, TestStorage>;

    type StorageProofResult = Result<
        (
            crate::StorageProof<<TestStorage as Storage>::Proof>,
            sov_rollup_interface::common::SlotNumber,
            <TestStorage as Storage>::Root,
        ),
        GetWithProofError,
    >;

    // Writes a block to the storage manager and finalizes it if requested.
    fn write_block(
        storage_manager: &mut TestStorageManager,
        prev_root: <TestStorage as Storage>::Root,
        da_header: &MockBlockHeader,
        user_key: &SlotKey,
        accessory_key: &SlotKey,
        value: &SlotValue,
        finalize: bool,
    ) -> <TestStorage as Storage>::Root {
        let (stf_storage, _ledger_storage) = storage_manager.create_state_for(da_header).unwrap();
        let (expected_root, mut state_update) = stf_storage
            .compute_state_update(
                StateAccesses {
                    user: OrderedReadsAndWrites {
                        ordered_reads: Vec::new(),
                        ordered_writes: vec![(user_key.clone(), Some(value.clone()))],
                    },
                    kernel: OrderedReadsAndWrites {
                        ordered_reads: Vec::new(),
                        ordered_writes: vec![(user_key.clone(), Some(value.clone()))],
                    },
                },
                &Default::default(),
                prev_root,
                None,
            )
            .unwrap();
        state_update.add_accessory_item(accessory_key.clone(), Some(value.clone()));

        storage_manager
            .save_change_set(
                da_header,
                stf_storage.materialize_changes(state_update),
                Default::default(),
            )
            .unwrap();

        if finalize {
            storage_manager.finalize(da_header).unwrap();
        }

        expected_root
    }

    fn assert_proof_matches_storage_and_accessory(
        storage: &TestStorage,
        user_key: &SlotKey,
        accessory_key: &SlotKey,
        expected_value: &SlotValue,
        expected_root: <TestStorage as Storage>::Root,
    ) {
        let (proof, slot_number, root_hash) = storage
            .get_with_proof_once::<User>(user_key.clone())
            .unwrap();

        assert_eq!(slot_number, storage.latest_version_unbound());
        assert_eq!(root_hash, expected_root);
        assert_eq!(proof.value, Some(expected_value.clone()));
        assert_eq!(
            TestStorage::open_proof(root_hash, proof.clone()).unwrap(),
            (user_key.clone(), Some(expected_value.clone()))
        );
        assert_eq!(
            storage.get_accessory_unbound(accessory_key.clone(), Some(slot_number)),
            Some(expected_value.clone())
        );
    }

    #[test]
    fn get_with_proof_reads_overlay_latest_user_and_accessory_state_across_multiple_blocks() {
        let tmpdir = tempfile::tempdir().unwrap();
        let mut storage_manager = TestStorageManager::new(
            RollupDbConfig::default_in_path(tmpdir.path().to_path_buf()),
            false,
        )
        .unwrap();
        let user_key = SlotKey::from_slice(b"user-counter");
        let accessory_key = SlotKey::from_slice(b"accessory-counter");
        let mut prev_root = TestStorage::PRE_GENESIS_ROOT;

        for height in 1..=3 {
            let da_header = MockBlockHeader::from_height(height);
            let expected_value = SlotValue::from(vec![height as u8]);
            let expected_root = write_block(
                &mut storage_manager,
                prev_root,
                &da_header,
                &user_key,
                &accessory_key,
                &expected_value,
                false, // don't finalize any blocks so that the overlay is newer than the DB
            );

            let (overlay_storage, _ledger_storage) =
                storage_manager.create_state_after(&da_header).unwrap();
            assert_proof_matches_storage_and_accessory(
                &overlay_storage,
                &user_key,
                &accessory_key,
                &expected_value,
                expected_root,
            );

            prev_root = expected_root;
        }
    }

    #[test]
    fn get_with_proof_reads_latest_committed_state_from_stale_storage() {
        let tmpdir = tempfile::tempdir().unwrap();
        let mut storage_manager = TestStorageManager::new(
            RollupDbConfig::default_in_path(tmpdir.path().to_path_buf()),
            false,
        )
        .unwrap();
        let user_key = SlotKey::from_slice(b"user-counter");
        let accessory_key = SlotKey::from_slice(b"accessory-counter");

        let first_header = MockBlockHeader::from_height(1);
        let first_root = write_block(
            &mut storage_manager,
            TestStorage::PRE_GENESIS_ROOT,
            &first_header,
            &user_key,
            &accessory_key,
            &SlotValue::from(vec![1]),
            true,
        );

        let (stale_storage, _ledger_storage) =
            storage_manager.create_state_after(&first_header).unwrap();

        let second_header = MockBlockHeader::from_height(2);
        let second_value = SlotValue::from(vec![2]);
        let expected_root = write_block(
            &mut storage_manager,
            first_root,
            &second_header,
            &user_key,
            &accessory_key,
            &second_value,
            true, // finalize blocks so that the DB gets newer than the "stale" storage overlay
        );

        assert!(stale_storage.latest_version() < stale_storage.latest_version_unbound());
        assert_proof_matches_storage_and_accessory(
            &stale_storage,
            &user_key,
            &accessory_key,
            &second_value,
            expected_root,
        );
    }

    /// This is the positive control for the normal committed-state path after block `N+1`
    /// finishes finalizing and we reopen storage at that new head. The
    /// `latest_version() == latest_version_unbound()` assertion checks that this handle is
    /// genuinely fresh rather than exercising stale-storage fallback behavior. The shared helper
    /// then verifies the full success path: `get_with_proof` returns the expected root and value,
    /// the proof opens successfully against that root, and accessory state at the returned slot
    /// matches the proven user value.
    #[test]
    fn get_with_proof_reads_latest_committed_state_from_fresh_storage() {
        let tmpdir = tempfile::tempdir().unwrap();
        let mut storage_manager = TestStorageManager::new(
            RollupDbConfig::default_in_path(tmpdir.path().to_path_buf()),
            false,
        )
        .unwrap();
        let user_key = SlotKey::from_slice(b"user-counter");
        let accessory_key = SlotKey::from_slice(b"accessory-counter");

        let first_header = MockBlockHeader::from_height(1);
        let first_root = write_block(
            &mut storage_manager,
            TestStorage::PRE_GENESIS_ROOT,
            &first_header,
            &user_key,
            &accessory_key,
            &SlotValue::from(vec![1]),
            true,
        );

        let second_header = MockBlockHeader::from_height(2);
        let second_value = SlotValue::from(vec![2]);
        let expected_root = write_block(
            &mut storage_manager,
            first_root,
            &second_header,
            &user_key,
            &accessory_key,
            &second_value,
            true,
        );

        let (fresh_storage, _ledger_storage) =
            storage_manager.create_state_after(&second_header).unwrap();
        assert_eq!(
            fresh_storage.latest_version(),
            fresh_storage.latest_version_unbound()
        );
        assert_proof_matches_storage_and_accessory(
            &fresh_storage,
            &user_key,
            &accessory_key,
            &second_value,
            expected_root,
        );
    }

    // These paused-finalization tests have two kinds of assertions.
    // `wait_until_wait_gate_reached`, the short `recv_timeout(...).is_err()` check, and the final
    // `latest_version() < latest_version_unbound()` assertion validate the test's setup
    // assumptions: finalization is paused at the intended spot, `get_with_proof` is blocked by
    // that in-flight finalization, and the handle is actually stale once the commit completes.
    // The real method-level correctness check is the post-release
    // `GetWithProofError::StateRootMismatch` result, which would be a bug if `get_with_proof`
    // ever turned into a successful proof or some
    // unrelated error here.
    fn run_get_with_proof_while_finalize_is_paused<F>(
        location: CommitFaultInjectionLocation,
        storage: &TestStorage,
        user_key: &SlotKey,
        finish_finalize: F,
    ) -> StorageProofResult
    where
        F: FnOnce(),
    {
        assert!(
            location.wait_until_wait_gate_reached(Duration::from_secs(5)),
            "timed out waiting for commit to pause at {location}"
        );

        let (proof_started_tx, proof_started_rx) = std::sync::mpsc::channel();
        let (proof_done_tx, proof_done_rx) = std::sync::mpsc::channel();
        let proof_thread = std::thread::spawn({
            let storage = storage.clone();
            let user_key = user_key.clone();
            move || {
                proof_started_tx.send(()).unwrap();
                let result = storage.get_with_proof_once::<User>(user_key);
                proof_done_tx.send(result).unwrap();
            }
        });

        proof_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("timed out waiting for proof request to start");
        assert!(
            proof_done_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "get_with_proof unexpectedly finished while commit was paused at {location}"
        );

        location.release_wait_gate();
        finish_finalize();
        let result = proof_done_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("timed out waiting for blocked proof request to finish");
        proof_thread.join().unwrap();
        result
    }

    fn run_paused_finalize_and_get_proof_result(
        location: CommitFaultInjectionLocation,
    ) -> (TestStorage, SlotKey, StorageProofResult) {
        let tmpdir = tempfile::tempdir().unwrap();
        let mut storage_manager = TestStorageManager::new(
            RollupDbConfig::default_in_path(tmpdir.path().to_path_buf()),
            false,
        )
        .unwrap();
        let user_key = SlotKey::from_slice(b"user-counter");
        let accessory_key = SlotKey::from_slice(b"accessory-counter");

        let first_header = MockBlockHeader::from_height(1);
        let first_root = write_block(
            &mut storage_manager,
            TestStorage::PRE_GENESIS_ROOT,
            &first_header,
            &user_key,
            &accessory_key,
            &SlotValue::from(vec![1]),
            true,
        );

        let (stale_storage, _ledger_storage) =
            storage_manager.create_state_after(&first_header).unwrap();

        location.arm_wait_gate();

        let second_header = MockBlockHeader::from_height(2);
        let commit_thread = std::thread::spawn({
            let user_key = user_key.clone();
            let accessory_key = accessory_key.clone();
            move || {
                write_block(
                    &mut storage_manager,
                    first_root,
                    &second_header,
                    &user_key,
                    &accessory_key,
                    &SlotValue::from(vec![2]),
                    true,
                )
            }
        });

        let result = run_get_with_proof_while_finalize_is_paused(
            location,
            &stale_storage,
            &user_key,
            move || {
                commit_thread.join().unwrap();
            },
        );

        assert!(stale_storage.latest_version() < stale_storage.latest_version_unbound());
        (stale_storage, accessory_key, result)
    }

    fn assert_stale_storage_blocks_then_observes_root_mismatch(
        location: CommitFaultInjectionLocation,
    ) {
        let (_stale_storage, _accessory_key, result) =
            run_paused_finalize_and_get_proof_result(location);
        let err = result.expect_err(
            "get_with_proof should observe a root mismatch once the paused commit finishes",
        );
        assert!(
            matches!(err, GetWithProofError::StateRootMismatch),
            "unexpected error after releasing paused commit: {err:?}"
        );
    }

    #[test]
    fn get_with_proof_waits_for_stale_storage_while_commit_is_paused_before_accessory() {
        assert_stale_storage_blocks_then_observes_root_mismatch(
            CommitFaultInjectionLocation::BeforeCommittingAccessory,
        );
    }

    #[test]
    fn get_with_proof_waits_for_stale_storage_while_commit_is_paused_before_ledger() {
        assert_stale_storage_blocks_then_observes_root_mismatch(
            CommitFaultInjectionLocation::BeforeCommittingLedger,
        );
    }

    #[test]
    fn get_with_proof_waits_for_stale_storage_while_commit_is_paused_before_archival() {
        assert_stale_storage_blocks_then_observes_root_mismatch(
            CommitFaultInjectionLocation::BeforeCommittingArchival,
        );
    }

    #[test]
    fn get_with_proof_waits_for_stale_storage_while_commit_is_paused_before_kernel_nomt() {
        assert_stale_storage_blocks_then_observes_root_mismatch(
            CommitFaultInjectionLocation::BeforeCommittingKernelNomt,
        );
    }

    #[test]
    fn get_with_proof_waits_for_stale_storage_while_commit_is_paused_before_user_nomt() {
        assert_stale_storage_blocks_then_observes_root_mismatch(
            CommitFaultInjectionLocation::BeforeCommittingUserNomt,
        );
    }

    #[test]
    fn get_with_proof_does_not_return_inconsistent_success_for_stale_storage_while_commit_is_paused_before_live(
    ) {
        let (stale_storage, accessory_key, result) = run_paused_finalize_and_get_proof_result(
            CommitFaultInjectionLocation::BeforeCommittingLive,
        );
        if let Ok((proof, slot_number, root_hash)) = result {
            assert_eq!(slot_number, stale_storage.latest_version_unbound());
            let (_, opened_value) =
                TestStorage::open_proof(root_hash, proof.clone()).expect("proof should verify");
            assert_eq!(opened_value, proof.value);
            assert_eq!(
                stale_storage.get_accessory_unbound(accessory_key, Some(slot_number)),
                proof.value
            );
        }
    }

    fn run_paused_finalize_and_get_overlay_proof_result(
        location: CommitFaultInjectionLocation,
    ) -> (
        TestStorage,
        SlotKey,
        SlotKey,
        SlotValue,
        <TestStorage as Storage>::Root,
        StorageProofResult,
    ) {
        let tmpdir = tempfile::tempdir().unwrap();
        let mut storage_manager = TestStorageManager::new(
            RollupDbConfig::default_in_path(tmpdir.path().to_path_buf()),
            false,
        )
        .unwrap();
        let user_key = SlotKey::from_slice(b"user-counter");
        let accessory_key = SlotKey::from_slice(b"accessory-counter");

        let first_header = MockBlockHeader::from_height(1);
        let first_root = write_block(
            &mut storage_manager,
            TestStorage::PRE_GENESIS_ROOT,
            &first_header,
            &user_key,
            &accessory_key,
            &SlotValue::from(vec![1]),
            true,
        );

        let second_header = MockBlockHeader::from_height(2);
        let expected_value = SlotValue::from(vec![2]);
        let expected_root = write_block(
            &mut storage_manager,
            first_root,
            &second_header,
            &user_key,
            &accessory_key,
            &expected_value,
            false,
        );
        let (overlay_storage, _ledger_storage) =
            storage_manager.create_state_after(&second_header).unwrap();

        location.arm_wait_gate();

        let finalize_thread = std::thread::spawn(move || storage_manager.finalize(&second_header));

        let result = run_get_with_proof_while_finalize_is_paused(
            location,
            &overlay_storage,
            &user_key,
            move || {
                finalize_thread.join().unwrap().unwrap();
            },
        );

        (
            overlay_storage,
            user_key,
            accessory_key,
            expected_value,
            expected_root,
            result,
        )
    }

    fn assert_overlay_storage_blocks_then_observes_consistent_success(
        location: CommitFaultInjectionLocation,
    ) {
        let (overlay_storage, user_key, accessory_key, expected_value, expected_root, result) =
            run_paused_finalize_and_get_overlay_proof_result(location);
        let (proof, slot_number, root_hash) =
            result.expect("get_with_proof should succeed once the paused commit finishes");

        assert_eq!(slot_number, overlay_storage.latest_version_unbound());
        assert_eq!(root_hash, expected_root);
        assert_eq!(proof.value, Some(expected_value.clone()));
        assert_eq!(
            TestStorage::open_proof(root_hash, proof.clone()).unwrap(),
            (user_key, Some(expected_value.clone()))
        );
        assert_eq!(
            overlay_storage.get_accessory_unbound(accessory_key, Some(slot_number)),
            Some(expected_value)
        );
    }

    #[test]
    fn get_with_proof_succeeds_for_overlay_storage_while_commit_is_paused_before_archival() {
        assert_overlay_storage_blocks_then_observes_consistent_success(
            CommitFaultInjectionLocation::BeforeCommittingArchival,
        );
    }

    #[test]
    fn get_with_proof_succeeds_for_overlay_storage_while_commit_is_paused_before_accessory() {
        assert_overlay_storage_blocks_then_observes_consistent_success(
            CommitFaultInjectionLocation::BeforeCommittingAccessory,
        );
    }

    #[test]
    fn get_with_proof_succeeds_for_overlay_storage_while_commit_is_paused_before_ledger() {
        assert_overlay_storage_blocks_then_observes_consistent_success(
            CommitFaultInjectionLocation::BeforeCommittingLedger,
        );
    }

    #[test]
    fn get_with_proof_succeeds_for_overlay_storage_while_commit_is_paused_before_kernel_nomt() {
        assert_overlay_storage_blocks_then_observes_consistent_success(
            CommitFaultInjectionLocation::BeforeCommittingKernelNomt,
        );
    }

    #[test]
    fn get_with_proof_succeeds_for_overlay_storage_while_commit_is_paused_before_user_nomt() {
        assert_overlay_storage_blocks_then_observes_consistent_success(
            CommitFaultInjectionLocation::BeforeCommittingUserNomt,
        );
    }

    #[test]
    fn get_with_proof_succeeds_for_overlay_storage_while_commit_is_paused_before_live() {
        assert_overlay_storage_blocks_then_observes_consistent_success(
            CommitFaultInjectionLocation::BeforeCommittingLive,
        );
    }
}
