use sha2::Digest;
use sov_db::ledger_db::LedgerDB;
use sov_mock_da::{
    MockAddress, MockBlockHeader, MockDaConfig, MockDaService, MockDaSpec, MockDaVerifier,
    MockValidityCond,
};
use sov_mock_zkvm::MockZkvm;
use sov_prover_storage_manager::{ProverStorageManager, SnapshotManager};
use sov_rollup_interface::da::{BlobReaderTrait, DaSpec};
use sov_rollup_interface::stf::{SlotResult, StateTransitionFunction};
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_rollup_interface::zk::{ValidityCondition, Zkvm};
use sov_state::storage::{StorageKey, StorageValue};
use sov_state::{
    ArrayWitness, DefaultStorageSpec, OrderedReadsAndWrites, Prefix, ProverStorage, Storage,
};
use sov_stf_runner::{
    InitVariant, ParallelProverService, ProverServiceConfig, RollupConfig, RollupProverConfig,
    RpcConfig, RunnerConfig, StateTransitionRunner, StorageConfig,
};

type MockInitVariant = InitVariant<HashStf<MockValidityCond>, MockZkvm, MockDaSpec>;

type S = DefaultStorageSpec;
type Q = SnapshotManager;

type StorageManager = ProverStorageManager<MockDaSpec, S>;

#[derive(Default, Clone)]
struct HashStf<Cond> {
    phantom_data: std::marker::PhantomData<Cond>,
}

impl<Cond> HashStf<Cond> {
    fn new() -> Self {
        Self {
            phantom_data: std::marker::PhantomData,
        }
    }

    fn hash_key() -> StorageKey {
        let prefix = Prefix::new(b"root".to_vec());
        StorageKey::singleton(&prefix)
    }

    fn save_from_hasher(
        hasher: sha2::Sha256,
        storage: ProverStorage<S, Q>,
        witness: &ArrayWitness,
    ) -> ([u8; 32], ProverStorage<S, Q>) {
        let result = hasher.finalize();

        let hash_key = HashStf::<Cond>::hash_key();
        let hash_value = StorageValue::from(result.as_slice().to_vec());

        let ordered_reads_writes = OrderedReadsAndWrites {
            ordered_reads: Vec::default(),
            ordered_writes: vec![(hash_key.to_cache_key(), Some(hash_value.into_cache_value()))],
        };

        let (jmt_root_hash, state_update) = storage
            .compute_state_update(ordered_reads_writes, witness)
            .unwrap();

        storage.commit(&state_update, &OrderedReadsAndWrites::default());

        let mut root_hash = [0u8; 32];

        for (i, &byte) in jmt_root_hash.as_ref().iter().enumerate().take(32) {
            root_hash[i] = byte;
        }

        (root_hash, storage)
    }
}

/// Outcome of the apply_slot method.

impl<Vm: Zkvm, Cond: ValidityCondition, Da: DaSpec> StateTransitionFunction<Vm, Da>
    for HashStf<Cond>
{
    type StateRoot = [u8; 32];
    type GenesisParams = Vec<u8>;
    type PreState = ProverStorage<S, Q>;
    type ChangeSet = ProverStorage<S, Q>;
    type TxReceiptContents = ();
    type BatchReceiptContents = [u8; 32];
    type Witness = ArrayWitness;
    type Condition = Cond;

    fn init_chain(
        &self,
        genesis_state: Self::PreState,
        params: Self::GenesisParams,
    ) -> (Self::StateRoot, Self::ChangeSet) {
        let mut hasher = sha2::Sha256::new();
        hasher.update(params);

        HashStf::<Cond>::save_from_hasher(hasher, genesis_state, &ArrayWitness::default())
    }

    fn apply_slot<'a, I>(
        &self,
        _pre_state_root: &Self::StateRoot,
        storage: Self::PreState,
        witness: Self::Witness,
        _slot_header: &Da::BlockHeader,
        _validity_condition: &Da::ValidityCondition,
        blobs: I,
    ) -> SlotResult<
        Self::StateRoot,
        Self::ChangeSet,
        Self::BatchReceiptContents,
        Self::TxReceiptContents,
        Self::Witness,
    >
    where
        I: IntoIterator<Item = &'a mut Da::BlobTransaction>,
    {
        let mut hasher = sha2::Sha256::new();

        let hash_key = HashStf::<Cond>::hash_key();
        let existing_cache = storage.get(&hash_key, None, &witness).unwrap();
        hasher.update(existing_cache.value());

        for blob in blobs {
            let data = blob.verified_data();
            hasher.update(data);
        }

        let (state_root, storage) = HashStf::<Cond>::save_from_hasher(hasher, storage, &witness);

        SlotResult {
            state_root,
            change_set: storage,
            // TODO: Add batch receipts to inspection
            batch_receipts: vec![],
            witness,
        }
    }
}

#[tokio::test]
async fn init_and_restart() {
    let tmpdir = tempfile::tempdir().unwrap();
    let genesis_params = vec![1, 2, 3, 4, 5];
    let init_variant: MockInitVariant = InitVariant::Genesis {
        block_header: MockBlockHeader::from_height(0),
        genesis_params,
    };

    let state_root_after_genesis = {
        let runner = initialize_runner(tmpdir.path(), init_variant);
        *runner.get_state_root()
    };

    let init_variant_2: MockInitVariant = InitVariant::Initialized(state_root_after_genesis);

    let runner_2 = initialize_runner(tmpdir.path(), init_variant_2);

    let state_root_2 = *runner_2.get_state_root();

    assert_eq!(state_root_after_genesis, state_root_2);
}

type MockProverService = ParallelProverService<
    [u8; 32],
    ArrayWitness,
    MockDaService,
    MockZkvm,
    HashStf<MockValidityCond>,
>;
fn initialize_runner(
    path: &std::path::Path,
    init_variant: MockInitVariant,
) -> StateTransitionRunner<
    HashStf<MockValidityCond>,
    StorageManager,
    MockDaService,
    MockZkvm,
    MockProverService,
> {
    let address = MockAddress::new([11u8; 32]);
    let rollup_config = RollupConfig::<MockDaConfig> {
        storage: StorageConfig {
            path: path.to_path_buf(),
        },
        runner: RunnerConfig {
            start_height: 1,
            rpc_config: RpcConfig {
                bind_host: "127.0.0.1".to_string(),
                bind_port: 0,
            },
        },
        da: MockDaConfig {
            sender_address: address,
        },
        prover_service: ProverServiceConfig {
            aggregated_proof_block_jump: 1,
        },
    };

    let da_service = MockDaService::new(address);

    let ledger_db = LedgerDB::with_path(path).unwrap();

    let stf = HashStf::<MockValidityCond>::new();

    let storage_config = sov_state::config::Config {
        path: path.to_path_buf(),
    };
    let mut storage_manager = ProverStorageManager::new(storage_config).unwrap();

    let vm = MockZkvm::default();
    let verifier = MockDaVerifier::default();

    let prover_config = RollupProverConfig::Prove;

    let prover_service = ParallelProverService::new(
        vm,
        stf.clone(),
        verifier,
        prover_config,
        // Should be ZkStorage, but we don't need it for this test
        storage_manager.create_finalized_storage().unwrap(),
        1,
        ProverServiceConfig {
            aggregated_proof_block_jump: 1,
        },
    );

    StateTransitionRunner::new(
        rollup_config.runner,
        da_service,
        ledger_db,
        stf,
        storage_manager,
        init_variant,
        prover_service,
    )
    .unwrap()
}
