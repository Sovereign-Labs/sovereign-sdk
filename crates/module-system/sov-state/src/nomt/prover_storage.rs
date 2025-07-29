//! Prover side of NOMT-based Storage implementation
use std::collections::btree_map::Entry;
use std::collections::BTreeMap;
use std::fmt::Formatter;

use anyhow::Context;
use nomt::hasher::BinaryHasher;
use nomt::proof::MultiProof;
use nomt::FinishedSession;
use sov_db::accessory_db::AccessoryDb;
use sov_db::historical_state::HistoricalStateReader;
use sov_db::namespaces::{KernelNamespace, UserNamespace};
use sov_db::state_db_nomt::NomtSessionBuilder;
use sov_db::storage_manager::{
    InitializableNativeNomtStorage, NomtChangeSet, StateFinishedSession,
};
use sov_rollup_interface::common::SlotNumber;
use sov_rollup_interface::reexports::digest::Digest;

use crate::storage::ReadType;
use crate::{
    Accessory, CompileTimeNamespace, MerkleProofSpec, Namespace, NativeStorage, NodeLeaf,
    NodeLeafAndMaybeValue, OrderedReadsAndWrites, ProvableCompileTimeNamespace, ProvableNamespace,
    SlotKey, SlotValue, StateAccesses, StateUpdate, Storage, StorageProof, StorageRoot, Witness,
};

type NomtSession<H> = nomt::Session<BinaryHasher<H>>;

/// A [`Storage`] implementation to be used by the prover in a native execution based on NOMT.
#[derive(derivative::Derivative)]
#[derivative(Clone(bound = "S: MerkleProofSpec"))]
pub struct NomtProverStorage<S: MerkleProofSpec, K>
where
    K: Clone,
{
    state_session_builder: NomtSessionBuilder<S::Hasher, K>,
    historical_state: HistoricalStateReader,
    accessory: AccessoryDb,
    /// If set to true, consistency between NOMT and rocksdb will be checked, in some cases.
    /// Please check [`NomtProverStorage::should_check_dbs_sync`] for more details.
    is_strict_mode: bool,
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
    /// If `strict_mode` is set to true, consistency between NOMT and rocksdb will be checked, in some cases.
    // Please check [`NomtProverStorage::should_check_dbs_sync`] for more details.
    pub fn create(
        state_session_builder: NomtSessionBuilder<S::Hasher, K>,
        historical_state: HistoricalStateReader,
        accessory: AccessoryDb,
        use_strict_mode: bool,
    ) -> Self {
        Self {
            state_session_builder,
            historical_state,
            accessory,
            is_strict_mode: use_strict_mode,
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
        self.is_strict_mode = use_strict_mode;
    }

    fn get_version_to_use(&self, version: Option<SlotNumber>) -> Option<SlotNumber> {
        if self.is_empty() {
            return None;
        }
        let next_version = self.historical_state.get_next_version();
        match version {
            None => Some(
                next_version
                    .checked_sub(1)
                    .expect("Next version for non empty storage should be above 0"),
            ),
            Some(passed_version) => {
                if passed_version >= next_version {
                    None
                } else {
                    Some(passed_version)
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
            self.is_strict_mode
            // latest version can be equal to genesis in 2 cases: pre-genesis and at genesis.
            // Since genesis is a special case and not covered by normal stf transition,
            // we exclude this case for simpler testing.
            && version_to_use > SlotNumber::GENESIS
            && version_to_use == self.latest_version()
    }

    fn read_value<N: CompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
    ) -> Option<SlotValue> {
        let resolved_version = self.get_version_to_use(version)?;
        let _span = tracing::debug_span!("version", %resolved_version, passed = ?version).entered();
        match N::NAMESPACE {
            Namespace::User => {
                let historical_value = self
                    .historical_state
                    .get_value_option_by_key::<UserNamespace>(resolved_version, key.as_ref())
                    .expect("Underlying user I/O failed");
                if self.should_check_dbs_sync(resolved_version) {
                    let key_path = S::Hasher::digest(key.as_ref()).into();
                    tracing::trace!(
                        %key,
                        key_path = hex::encode(key_path),
                        "Reading from user namespace",
                    );
                    let nomt_session = self
                        .state_session_builder
                        .begin_user_session()
                        .expect("Failed to build user session");
                    let nomt_value = nomt_session.read(key_path).unwrap();
                    drop(nomt_session);
                    let historical_value_hash = historical_value.as_ref().map(|v| {
                        SlotValue::from(v.to_vec()).combine_val_hash_and_size::<S::Hasher>()
                    });
                    assert_eq!(nomt_value, historical_value_hash);
                }

                historical_value
            }
            Namespace::Kernel => {
                let historical_value = self
                    .historical_state
                    .get_value_option_by_key::<KernelNamespace>(resolved_version, key.as_ref())
                    .expect("Underlying user I/O failed");
                if self.should_check_dbs_sync(resolved_version) {
                    let key_path = S::Hasher::digest(key.as_ref()).into();
                    tracing::trace!(
                        %key,
                        key_path = hex::encode(key_path),
                        "Reading from kernel namespace",
                    );
                    let nomt_session = self
                        .state_session_builder
                        .begin_kernel_session()
                        .expect("Failed to build kernel session");
                    let nomt_value = nomt_session.read(key_path).unwrap();
                    drop(nomt_session);
                    let historical_value_hash = historical_value.as_ref().map(|v| {
                        SlotValue::from(v.to_vec()).combine_val_hash_and_size::<S::Hasher>()
                    });
                    assert_eq!(nomt_value, historical_value_hash);
                }

                historical_value
            }
            Namespace::Accessory => self
                .accessory
                .get_value_option(key.as_ref(), resolved_version)
                .expect("Unable to read from AccessoryDb"),
        }
        .map(Into::into)
    }

    fn do_get_leaf<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        witness: Option<&<Self as Storage>::Witness>,
    ) -> Option<NodeLeafAndMaybeValue> {
        let val = self.read_value::<N>(key, version);

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
        node_leaf_with_fetched_value
    }
}

fn to_nomt_accesses<S: MerkleProofSpec>(
    session: &NomtSession<S::Hasher>,
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
        // From documentation:
        // > This should be called for every logical write within the session, as well as every
        // > logical read if you expect to generate a merkle proof for the session.
        // So warming up all reads.
        session.warm_up(key_hash);

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
        session.warm_up(key_hash);

        let authenticated_write = original_write
            .as_ref()
            .map(|v| v.combine_val_hash_and_size::<S::Hasher>());

        tracing::trace!(
            %key,
            key_path = hex::encode(key_hash),
            original_write = ?original_write
                .as_ref()
                .map(|v| String::from_utf8_lossy(v.value())),
            authenticated_write = ?authenticated_write.as_ref().map(hex::encode),
            "state update write",
        );

        match merged_accesses.entry(key_hash) {
            Entry::Vacant(vacant) => {
                // Also warming up all writes. `ReadThenWrite` has been warmed up during reads collection.
                session.warm_up(key_hash);
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
    accesses: &OrderedReadsAndWrites,
    witness: &S::Witness,
) -> anyhow::Result<FinishedSession> {
    tracing::trace!(
        reads = accesses.ordered_reads.len(),
        writes = accesses.ordered_writes.len(),
        "compute state update"
    );
    let nomt_accesses = to_nomt_accesses::<S>(&session, accesses)?;
    let mut finished = session.finish(nomt_accesses)?;
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
        use_strict_mode: bool,
    ) -> Self {
        Self::create(state_db, historical_state, accessory_db, use_strict_mode)
    }
}

#[allow(missing_docs)]
pub struct NomtStateUpdate<S: MerkleProofSpec> {
    user: FinishedSession,
    kernel: FinishedSession,
    accessory: OrderedReadsAndWrites,
    state_accesses: StateAccesses,
    next_root_hash: StorageRoot<S>,
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
    type Proof = ();
    type Root = StorageRoot<S>;
    // These 2 are effectively the same thing, `StateUpdate` is not materialized, `ChangeSet` is materialized.
    type StateUpdate = NomtStateUpdate<S>;
    type ChangeSet = NomtChangeSet;
    const PRE_GENESIS_ROOT: Self::Root =
        StorageRoot::new(nomt::trie::TERMINATOR, nomt::trie::TERMINATOR);

    fn put_in_witness(&self, value: Option<SlotValue>, witness: &Self::Witness) {
        witness.add_hint(&value);
    }

    fn get_leaf<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        self.do_get_leaf::<N>(key, None, Some(witness))
    }

    fn get<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        witness: &Self::Witness,
    ) -> Option<SlotValue> {
        let val = self.read_value::<N>(key, None);
        witness.add_hint(&val);
        val
    }

    fn get_accessory(&self, key: &SlotKey, version: Option<SlotNumber>) -> Option<SlotValue> {
        self.read_value::<Accessory>(key, version)
    }

    fn compute_state_update(
        &self,
        state_accesses: StateAccesses,
        witness: &Self::Witness,
        prev_state_root: Self::Root,
    ) -> anyhow::Result<(Self::Root, Self::StateUpdate)> {
        let start = std::time::Instant::now();
        let next_version = self.historical_state.get_next_version();
        tracing::trace!(%prev_state_root, %next_version, "NomtProverStorage, computing state update");
        // Open 2 sessions close to each other
        let user_session = self.state_session_builder.begin_user_session()?;
        let kernel_session = self.state_session_builder.begin_kernel_session()?;

        let current_prev_user_root = user_session.prev_root().into_inner();
        let current_prev_kernel_root = kernel_session.prev_root().into_inner();
        let current_prev_root = StorageRoot::new(current_prev_user_root, current_prev_kernel_root);

        // Check staleness, pre-computation:
        if self.is_strict_mode && current_prev_root != prev_state_root {
            anyhow::bail!("stale storage on next_version={}, passed prev_state_root {} does not match the current prev_state_root {}",
                next_version,
                prev_state_root,
                current_prev_root
            );
        }

        let user_finished_session = {
            let _span = tracing::debug_span!("compute_state_update", namespace = "user").entered();
            compute_state_update_namespace::<S>(user_session, &state_accesses.user, witness)
                .context("user state")?
        };
        let kernel_finished_session = {
            let _span =
                tracing::debug_span!("compute_state_update", namespace = "kernel").entered();
            compute_state_update_namespace::<S>(kernel_session, &state_accesses.kernel, witness)
                .context("kernel state")?
        };

        // Additional self-check that the finished session has the same previous root hash as passed prev_state_root.
        let kernel_finished_session_prev_root = kernel_finished_session.prev_root().into_inner();
        let user_finished_session_prev_root = user_finished_session.prev_root().into_inner();
        let finished_session_prev_root = StorageRoot::new(
            user_finished_session_prev_root,
            kernel_finished_session_prev_root,
        );

        // Check staleness, post-computation. This should check if storage became stale during the computation.
        if self.is_strict_mode && prev_state_root != finished_session_prev_root {
            anyhow::bail!("stale storage on next_version={}, passed prev_state_root {} does not match the current prev_state_root {}",
                next_version,
                prev_state_root,
                current_prev_root
            );
        }

        let user_root = user_finished_session.root();
        let kernel_root = kernel_finished_session.root();
        let root = StorageRoot::new(user_root.into_inner(), kernel_root.into_inner());

        tracing::debug!(state_root = %root, %next_version, time = ?start.elapsed(), "computed next state root");

        let state_update = NomtStateUpdate {
            user: user_finished_session,
            kernel: kernel_finished_session,
            accessory: Default::default(),
            state_accesses,
            next_root_hash: root,
        };

        Ok((root, state_update))
    }

    fn materialize_changes(self, state_update: Self::StateUpdate) -> Self::ChangeSet {
        let next_version = self.historical_state.get_next_version();
        tracing::trace!(%next_version, "NomtProverStorage, materializing changes");
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
        } = state_update;
        let user_to_materialize = user_versioned.ordered_writes.into_iter().map(|(k, v)| {
            // TODO: Clone now, figure out how to optimize later
            (k.as_ref().clone(), v.map(|x| x.value().to_vec()))
        });
        let kernel_to_materialize = kernel_versioned.ordered_writes.into_iter().map(|(k, v)| {
            // TODO: Clone now, figure out how to optimize later
            (k.as_ref().clone(), v.map(|x| x.value().to_vec()))
        });
        let historical_schema_batch = HistoricalStateReader::materialize_values(
            user_to_materialize,
            kernel_to_materialize,
            borsh::to_vec(&next_root_hash).expect("Failed to serialize root hash"),
            next_version,
        )
        .expect("historical state db materialization must succeed");
        let accessory_batch = AccessoryDb::materialize_values(
            accessory_writes
                .ordered_writes
                .iter()
                .map(|(k, v_opt)| (k.key().to_vec(), v_opt.as_ref().map(|v| v.value().to_vec()))),
            next_version,
        )
        .expect("accessory db materialization must succeed");
        NomtChangeSet {
            state: StateFinishedSession::new(user, kernel),
            historical_state: historical_schema_batch,
            accessory: accessory_batch,
        }
    }

    fn open_proof(
        _state_root: Self::Root,
        _proof: StorageProof<Self::Proof>,
    ) -> anyhow::Result<(SlotKey, Option<SlotValue>)> {
        unimplemented!("The NomtProverStorage does not support `open_proof` yet.")
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
    ) -> Option<SlotValue> {
        self.read_value::<N>(key, version)
    }

    fn get_leaf_historical<N: ProvableCompileTimeNamespace>(
        &self,
        key: &SlotKey,
        version: Option<SlotNumber>,
        _witness: &Self::Witness,
    ) -> Option<NodeLeafAndMaybeValue> {
        self.do_get_leaf::<N>(key, version, None)
    }

    fn get_with_proof<N: ProvableCompileTimeNamespace>(
        &self,
        key: SlotKey,
        slot_number: Option<SlotNumber>,
    ) -> anyhow::Result<StorageProof<Self::Proof>> {
        let version_to_use = match self.get_version_to_use(slot_number) {
            None => {
                anyhow::bail!(
                    "Proof is not available at version {:?}. Empty storage or future version",
                    slot_number,
                )
            }
            Some(v) => v,
        };
        let namespace = N::PROVABLE_NAMESPACE;
        let value = match namespace {
            ProvableNamespace::User => self.read_value::<crate::User>(&key, Some(version_to_use)),
            ProvableNamespace::Kernel => {
                self.read_value::<crate::Kernel>(&key, Some(version_to_use))
            }
        };

        Ok(StorageProof {
            key,
            value,
            // TODO: Proof is empty now, will be fixed in follow
            proof: (),
            namespace,
        })
    }

    fn get_root_hash(&self, version: SlotNumber) -> anyhow::Result<Self::Root> {
        let version_to_use = match self.get_version_to_use(Some(version)) {
            None => {
                // Mimic error from jmt, historical reasons.
                anyhow::bail!("Root node not found for version {}.", version)
            }
            Some(v) => v,
        };
        let storage_root_historical = self.get_root_hash_unbound(version_to_use)?;
        if self.should_check_dbs_sync(version_to_use) {
            let user_session = self.state_session_builder.begin_user_session()?;
            let user_root = user_session.prev_root();
            drop(user_session);
            let kernel_session = self.state_session_builder.begin_kernel_session()?;
            let kernel_root = kernel_session.prev_root();
            drop(kernel_session);
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
}
