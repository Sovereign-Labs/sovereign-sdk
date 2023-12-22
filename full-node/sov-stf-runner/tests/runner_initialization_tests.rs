use sov_db::ledger_db::LedgerDB;
use sov_mock_da::{
    MockAddress, MockBlockHeader, MockDaConfig, MockDaService, MockDaSpec, MockDaVerifier,
    MockValidityCond,
};
use sov_mock_zkvm::MockZkvm;
use sov_prover_storage_manager::ProverStorageManager;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::{ArrayWitness, DefaultStorageSpec};
use sov_stf_runner::{
    InitVariant, ParallelProverService, ProverServiceConfig, RollupConfig, RollupProverConfig,
    RpcConfig, RunnerConfig, StateTransitionRunner, StorageConfig,
};

mod hash_stf;

use hash_stf::HashStf;

type MockInitVariant =
    InitVariant<HashStf<MockValidityCond>, MockZkvm<MockValidityCond>, MockDaSpec>;

type S = DefaultStorageSpec;
type StorageManager = ProverStorageManager<MockDaSpec, S>;

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
    MockZkvm<MockValidityCond>,
    HashStf<MockValidityCond>,
>;
fn initialize_runner(
    path: &std::path::Path,
    init_variant: MockInitVariant,
) -> StateTransitionRunner<
    HashStf<MockValidityCond>,
    StorageManager,
    MockDaService,
    MockZkvm<MockValidityCond>,
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

    let vm = MockZkvm::new(MockValidityCond::default());
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
        rollup_config.prover_service,
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
