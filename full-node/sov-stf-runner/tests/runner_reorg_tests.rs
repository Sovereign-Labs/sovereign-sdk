use sov_mock_da::{
    MockAddress, MockBlob, MockBlock, MockBlockHeader, MockDaConfig, MockDaService, MockDaSpec,
    MockDaVerifier, MockValidityCond, PlannedFork,
};
use sov_mock_zkvm::MockZkvm;
use sov_stf_runner::{
    InitVariant, ParallelProverService, ProverServiceConfig, RollupConfig, RollupProverConfig,
    RpcConfig, RunnerConfig, StateTransitionRunner, StorageConfig,
};

mod hash_stf;

use hash_stf::{get_result_from_blocks, HashStf, Q, S};
use sov_db::ledger_db::LedgerDB;
use sov_prover_storage_manager::ProverStorageManager;
use sov_rollup_interface::services::da::DaService;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::storage::NativeStorage;
use sov_state::{ProverStorage, Storage};

type MockInitVariant =
    InitVariant<HashStf<MockValidityCond>, MockZkvm<MockValidityCond>, MockDaSpec>;
#[tokio::test]
async fn test_simple_reorg_case() {
    let tmpdir = tempfile::tempdir().unwrap();
    let sequencer_address = MockAddress::new([11u8; 32]);
    let genesis_params = vec![1, 2, 3, 4, 5];

    let main_chain_blobs = vec![
        vec![1, 1, 1, 1],
        vec![2, 2, 2, 2],
        vec![3, 3, 3, 3],
        vec![4, 4, 4, 4],
    ];
    let fork_blobs = vec![
        vec![13, 13, 13, 13],
        vec![14, 14, 14, 14],
        vec![15, 15, 15, 15],
    ];
    let expected_final_blobs = vec![
        vec![1, 1, 1, 1],
        vec![2, 2, 2, 2],
        vec![13, 13, 13, 13],
        vec![14, 14, 14, 14],
        vec![15, 15, 15, 15],
    ];

    let mut da_service = MockDaService::with_finality(sequencer_address, 4);
    da_service.set_wait_attempts(2);

    let genesis_header = da_service.get_last_finalized_block_header().await.unwrap();

    let planned_fork = PlannedFork::new(5, 2, fork_blobs.clone());
    da_service.set_planned_fork(planned_fork).await.unwrap();

    for b in &main_chain_blobs {
        da_service.send_transaction(b).await.unwrap();
    }

    let (expected_state_root, _expected_final_root_hash) =
        get_expected_execution_hash_from(&genesis_params, expected_final_blobs);
    let (_expected_committed_state_root, expected_committed_root_hash) =
        get_expected_execution_hash_from(&genesis_params, vec![vec![1, 1, 1, 1]]);

    let init_variant: MockInitVariant = InitVariant::Genesis {
        block_header: genesis_header,
        genesis_params,
    };

    let (before, after) = runner_execution(tmpdir.path(), init_variant, da_service).await;
    assert_ne!(before, after);
    assert_eq!(expected_state_root, after);

    let committed_root_hash = get_saved_root_hash(tmpdir.path()).unwrap().unwrap();

    assert_eq!(expected_committed_root_hash.unwrap(), committed_root_hash);
}

#[tokio::test]
#[ignore = "TBD"]
async fn test_several_reorgs() {}

#[tokio::test]
async fn test_instant_finality_data_stored() {
    let tmpdir = tempfile::tempdir().unwrap();
    let sequencer_address = MockAddress::new([11u8; 32]);
    let genesis_params = vec![1, 2, 3, 4, 5];

    let mut da_service = MockDaService::new(sequencer_address);
    da_service.set_wait_attempts(2);

    let genesis_header = da_service.get_last_finalized_block_header().await.unwrap();

    da_service.send_transaction(&[1, 1, 1, 1]).await.unwrap();
    da_service.send_transaction(&[2, 2, 2, 2]).await.unwrap();
    da_service.send_transaction(&[3, 3, 3, 3]).await.unwrap();

    let (expected_state_root, expected_root_hash) = get_expected_execution_hash_from(
        &genesis_params,
        vec![vec![1, 1, 1, 1], vec![2, 2, 2, 2], vec![3, 3, 3, 3]],
    );

    let init_variant: MockInitVariant = InitVariant::Genesis {
        block_header: genesis_header,
        genesis_params,
    };

    let (before, after) = runner_execution(tmpdir.path(), init_variant, da_service).await;
    assert_ne!(before, after);
    assert_eq!(expected_state_root, after);

    let saved_root_hash = get_saved_root_hash(tmpdir.path()).unwrap().unwrap();

    assert_eq!(expected_root_hash.unwrap(), saved_root_hash);
}

async fn runner_execution(
    path: &std::path::Path,
    init_variant: MockInitVariant,
    da_service: MockDaService,
) -> ([u8; 32], [u8; 32]) {
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
            sender_address: da_service.get_sequencer_address(),
        },
        prover_service: ProverServiceConfig {
            aggregated_proof_block_jump: 1,
        },
    };

    let ledger_db = LedgerDB::with_path(path).unwrap();

    let stf = HashStf::<MockValidityCond>::new();

    let storage_config = sov_state::config::Config {
        path: rollup_config.storage.path.clone(),
    };
    let mut storage_manager = ProverStorageManager::new(storage_config).unwrap();

    let vm = MockZkvm::new(MockValidityCond::default());
    let verifier = MockDaVerifier::default();
    let prover_config = RollupProverConfig::Skip;

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

    let mut runner = StateTransitionRunner::new(
        rollup_config.runner,
        da_service,
        ledger_db,
        stf,
        storage_manager,
        init_variant,
        prover_service,
    )
    .unwrap();

    let before = *runner.get_state_root();
    let end = runner.run_in_process().await;
    assert!(end.is_err());
    let after = *runner.get_state_root();

    (before, after)
}

fn get_saved_root_hash(
    path: &std::path::Path,
) -> anyhow::Result<Option<<ProverStorage<S, Q> as Storage>::Root>> {
    let storage_config = sov_state::config::Config {
        path: path.to_path_buf(),
    };
    let mut storage_manager = ProverStorageManager::<MockDaSpec, S>::new(storage_config).unwrap();
    let finalized_storage = storage_manager.create_finalized_storage()?;

    let ledger_db = LedgerDB::with_path(path).unwrap();

    ledger_db
        .get_head_slot()?
        .map(|(number, _)| finalized_storage.get_root_hash(number.0))
        .transpose()
}

fn get_expected_execution_hash_from(
    genesis_params: &[u8],
    blobs: Vec<Vec<u8>>,
) -> ([u8; 32], Option<<ProverStorage<S, Q> as Storage>::Root>) {
    let blocks: Vec<MockBlock> = blobs
        .into_iter()
        .enumerate()
        .map(|(idx, blob)| MockBlock {
            header: MockBlockHeader::from_height((idx + 1) as u64),
            validity_cond: MockValidityCond::default(),
            blobs: vec![MockBlob::new(
                blob,
                MockAddress::new([11u8; 32]),
                [idx as u8; 32],
            )],
        })
        .collect();

    get_result_from_blocks(genesis_params, &blocks[..])
}
