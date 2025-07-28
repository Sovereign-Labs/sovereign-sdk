use std::sync::Arc;

use anyhow::Context;
use sov_db::ledger_db::LedgerDb;
use sov_db::storage_manager::NativeStorageManager;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::{
    BlockProducingConfig, MockAddress, MockBlob, MockBlock, MockBlockHeader, MockDaConfig,
    MockDaService, MockDaSpec, PlannedFork, RandomizationBehaviour, RandomizationConfig,
};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::provable_height_tracker::InfiniteHeight;
use sov_modules_api::{FullyBakedTx, StateTransitionFunction};
use sov_rollup_interface::common::HexHash;
use sov_rollup_interface::da::BlockHeaderTrait;
use sov_rollup_interface::node::da::{DaService, SlotData};
use sov_rollup_interface::node::SyncStatus;
use sov_rollup_interface::storage::HierarchicalStorageManager;
use sov_state::storage::NativeStorage;
use sov_state::{ArrayWitness, ProverStorage, Storage, StorageRoot};
use sov_stf_runner::StateTransitionRunner;
use sov_test_utils::storage::SimpleStorageManager;
use tempfile::TempDir;
use tokio::sync::watch;

use crate::helpers::hash_stf::{HashStf, S};
use crate::helpers::runner_init::{
    bootstrap_state_update_info, initialize_runner, HashStfRunner, InitVariant,
};

type MockInitVariant = InitVariant<HashStf, MockZkvm, MockZkvm, MockDaService>;

const STANDARD_SENDER: MockAddress = MockAddress::new([0u8; 32]);
const TREE_MINUTES: std::time::Duration = std::time::Duration::from_secs(60 * 3);

#[tokio::test(flavor = "multi_thread")]
async fn test_simple_reorg_case() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let sequencer_address = MockAddress::new([11u8; 32]);
    let genesis_params = vec![1, 2, 3, 4, 5];

    let main_chain_blobs = vec![
        batch(vec![1, 1, 1, 1]),
        batch(vec![2, 2, 2, 2]),
        batch(vec![3, 3, 3, 3]),
        batch(vec![4, 4, 4, 4]),
    ];
    let fork_blobs = vec![
        batch(vec![13, 13, 13, 13]),
        batch(vec![14, 14, 14, 14]),
        batch(vec![15, 15, 15, 15]),
    ];
    let expected_final_blobs = vec![
        batch(vec![1, 1, 1, 1]),
        batch(vec![2, 2, 2, 2]),
        batch(vec![13, 13, 13, 13]),
        batch(vec![14, 14, 14, 14]),
        batch(vec![15, 15, 15, 15]),
    ];

    let mut da_service = MockDaService::new(sequencer_address)
        .with_finality(4)
        .with_wait_attempts(2);

    let genesis_block = da_service.get_block_at(0).await.unwrap();

    let planned_fork = PlannedFork::new(5, 2, fork_blobs.clone());
    da_service.set_planned_fork(planned_fork).await.unwrap();

    let da_service = Arc::new(da_service);
    for data in main_chain_blobs {
        da_service
            .send_transaction(&data)
            .await
            .await
            .unwrap()
            .unwrap();
    }

    let (expected_state_root, _expected_final_root_hash) =
        get_expected_execution_hash_from(&genesis_params, expected_final_blobs);

    let (_expected_committed_state_root, expected_committed_root_hash) =
        get_expected_execution_hash_from(&genesis_params, vec![batch(vec![1, 1, 1, 1])]);

    let init_variant: MockInitVariant = InitVariant::Genesis {
        block: genesis_block,
        genesis_params,
    };

    check_runner(da_service, &tmp_dir, init_variant, expected_state_root).await;

    let committed_root_hash = get_saved_root_hash(tmp_dir.path()).unwrap().unwrap();
    assert_eq!(expected_committed_root_hash, committed_root_hash);
}

async fn test_runner_with_background_da_service(
    target_height: u64,
    da_config: MockDaConfig,
) -> anyhow::Result<()> {
    // std::env::set_var("RUST_LOG", "info,sov_stf_runner=trace,sov_mock_da=debug");
    // std::env::set_var("RUST_LOG", "info");
    // sov_test_utils::initialize_logging();
    let (shutdown_sender, mut shutdown_receiver) = watch::channel(());
    shutdown_receiver.mark_unchanged();

    let da_service =
        StorableMockDaService::from_config(da_config.clone(), shutdown_receiver.clone()).await;
    let da_service = Arc::new(da_service);
    let tempdir = tempfile::tempdir()?;
    let finality = da_config.finalization_blocks;
    let rollup_config = crate::helpers::runner_init::rollup_config_with_da::<StorableMockDaService>(
        tempdir.path(),
        da_config,
        1,
    );

    let stf = HashStf::new();

    let mut storage_manager: crate::helpers::runner_init::StorageManager =
        NativeStorageManager::new(tempdir.path())?;

    let (state_update_sender, _state_update_recv) =
        watch::channel(bootstrap_state_update_info(&mut storage_manager).await?);
    let (sync_sender, mut sync_status_receiver) = watch::channel(SyncStatus::START);
    sync_status_receiver.mark_unchanged();

    let genesis_params = vec![1, 2, 3, 4, 5];
    let block = da_service.get_block_at(0).await?;
    let genesis_header = block.header().clone();
    let init_variant: MockInitVariant = InitVariant::Genesis {
        block,
        genesis_params,
    };
    let (prev_state_root, _genesis_state_root) =
        init_variant.initialize(&stf, &mut storage_manager).await?;

    let (_, ledger_state) = storage_manager.create_state_after(&genesis_header).unwrap();
    let ledger_db = LedgerDb::with_reader(ledger_state).unwrap();
    let mut runner: HashStfRunner<StorableMockDaService> = StateTransitionRunner::new(
        rollup_config.runner.clone(),
        None,
        da_service.clone(),
        ledger_db.clone(),
        stf,
        storage_manager,
        state_update_sender,
        prev_state_root,
        sync_sender,
        Box::new(InfiniteHeight),
        shutdown_receiver.clone(),
        rollup_config.monitoring.clone(),
        None,
        None,
    )
    .await?;

    let runner_task = tokio::spawn(async move {
        runner.run_in_process().await.map_err(|error| {
            tracing::warn!(?error, "Runner return execution with error");
            error
        })
    });

    let mut synced_da_height = 0;
    // TODO: Adjust this to be more realistic and with actual motivation
    let seen_da_height_boundary = target_height + finality as u64 + 30;

    while synced_da_height <= target_height {
        let batch = vec![FullyBakedTx {
            data: vec![1, 2, 3],
        }];

        let serialized_batch = borsh::to_vec(&batch)?;
        let _ = da_service.send_transaction(&serialized_batch).await.await?;

        sync_status_receiver.changed().await?;

        let sync_status = { *sync_status_receiver.borrow() };

        synced_da_height = match sync_status {
            SyncStatus::Synced { synced_da_height }
            | SyncStatus::Syncing {
                synced_da_height, ..
            } => synced_da_height,
        };

        let head = da_service.get_head_block_header().await?;
        if head.height() > seen_da_height_boundary {
            anyhow::bail!("Runner didn't manage to sync in time.");
        }
    }

    shutdown_sender.send(())?;
    runner_task
        .await?
        .context("Runner did not completed with success")?;

    Ok(())
}

fn build_da_config(
    finality: u32,
    block_time_ms: u64,
    randomization: RandomizationConfig,
) -> MockDaConfig {
    let block_producing = BlockProducingConfig::Periodic { block_time_ms };
    MockDaConfig {
        connection_string: MockDaConfig::sqlite_in_memory(),
        sender_address: STANDARD_SENDER,
        finalization_blocks: finality,
        block_producing,
        da_layer: None,
        randomization: Some(randomization),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn flaky_test_runner_multiple_reorg_shuffle() -> anyhow::Result<()> {
    let finality = 50;
    let block_time_ms = 500;
    let randomization = RandomizationConfig {
        seed: HexHash::from([1; 32]),
        reorg_interval: 1..3,
        // TODO: It also messes up things with shorer block_time. get back to this later
        behaviour: RandomizationBehaviour::only_shuffle(20),
    };
    let da_config = build_da_config(finality, block_time_ms, randomization);

    tokio::time::timeout(
        TREE_MINUTES,
        test_runner_with_background_da_service(40, da_config),
    )
    .await?
}

#[tokio::test(flavor = "multi_thread")]
async fn test_runner_multiple_reorg_with_rewind() -> anyhow::Result<()> {
    let finality = 20;
    let block_time_ms = 400;
    let randomization = RandomizationConfig {
        seed: HexHash::from([1; 32]),
        reorg_interval: 1..3,
        behaviour: RandomizationBehaviour::ShuffleAndResize {
            drop_percent: 10,
            adjust_head_height: -15..15,
        },
    };
    let da_config = build_da_config(finality, block_time_ms, randomization);

    tokio::time::timeout(
        TREE_MINUTES,
        test_runner_with_background_da_service(40, da_config),
    )
    .await?
}

#[tokio::test(flavor = "multi_thread")]
async fn test_instant_finality_data_stored() -> anyhow::Result<()> {
    let tmp_dir = tempfile::tempdir()?;
    let sequencer_address = MockAddress::new([11u8; 32]);
    let genesis_params = vec![1, 2, 3, 4, 5];

    let da_service = Arc::new(MockDaService::new(sequencer_address).with_wait_attempts(2));

    let genesis_block = da_service.get_block_at(0).await?;

    let serialized_blob_1 = batch(vec![1, 1, 1, 1]);

    da_service
        .send_transaction(&serialized_blob_1)
        .await
        .await??;

    let serialized_blob_2 = batch(vec![2, 2, 2, 2]);

    da_service
        .send_transaction(&serialized_blob_2)
        .await
        .await??;

    let serialized_blob_3 = batch(vec![3, 3, 3, 3]);

    da_service
        .send_transaction(&serialized_blob_3)
        .await
        .await??;

    let (expected_state_root, expected_root_hash) = get_expected_execution_hash_from(
        &genesis_params,
        vec![serialized_blob_1, serialized_blob_2, serialized_blob_3],
    );

    let init_variant: MockInitVariant = InitVariant::Genesis {
        block: genesis_block,
        genesis_params,
    };

    check_runner(da_service, &tmp_dir, init_variant, expected_state_root).await;

    let saved_root_hash = get_saved_root_hash(tmp_dir.path()).unwrap().unwrap();
    assert_eq!(expected_root_hash, saved_root_hash);
    Ok(())
}

async fn check_runner(
    da_service: Arc<MockDaService>,
    tmpdir: &TempDir,
    init_variant: MockInitVariant,
    expected_state_root: StorageRoot<S>,
) {
    let (mut runner, test_node) =
        initialize_runner(da_service, tmpdir.path(), init_variant, 1, None).await;
    let before = *runner.get_state_root();
    let end = runner.run_in_process().await;
    // TODO: Subscribe to block notifications and shutdown runner afterwards.
    assert!(end.is_err());
    let after = *runner.get_state_root();

    assert_ne!(before, after);
    assert_eq!(expected_state_root, after);
    test_node.stop().await;
}

fn get_saved_root_hash(
    path: &std::path::Path,
) -> anyhow::Result<Option<<ProverStorage<S> as Storage>::Root>> {
    let mut storage_manager =
        NativeStorageManager::<MockDaSpec, ProverStorage<S>>::new(path).unwrap();
    let mock_block_header = MockBlockHeader::from_height(1000000);
    let (stf_state, ledger_state) = storage_manager.create_state_for(&mock_block_header)?;

    let ledger_db = LedgerDb::with_reader(ledger_state).unwrap();

    ledger_db
        .get_head_slot()?
        .map(|(number, _)| stf_state.get_root_hash(number))
        .transpose()
}

fn get_expected_execution_hash_from(
    genesis_params: &[u8],
    blobs: Vec<Vec<u8>>,
) -> (StorageRoot<S>, <ProverStorage<S> as Storage>::Root) {
    let blocks: Vec<MockBlock> = blobs
        .into_iter()
        .enumerate()
        .map(|(idx, blob)| MockBlock {
            header: MockBlockHeader::from_height((idx + 1) as u64),
            batch_blobs: vec![MockBlob::new(
                blob,
                MockAddress::new([11u8; 32]),
                [idx as u8; 32],
            )],
            proof_blobs: Default::default(),
        })
        .collect();

    get_result_from_blocks(genesis_params, &blocks[..])
}

// Returns final data hash and root hash
fn get_result_from_blocks(
    genesis_params: &[u8],
    blocks: &[MockBlock],
) -> (StorageRoot<S>, <ProverStorage<S> as Storage>::Root) {
    let mut storage_manager = SimpleStorageManager::new();
    let storage = storage_manager.create_storage();

    let stf = HashStf::new();

    let (genesis_state_root, change_set) =
        <HashStf as StateTransitionFunction<MockZkvm, MockZkvm, MockDaSpec>>::init_chain(
            &stf,
            &Default::default(),
            storage,
            genesis_params.to_vec(),
        );
    storage_manager.commit(change_set);

    let mut state_root = genesis_state_root;

    for block in blocks {
        let mut relevant_blobs = block.as_relevant_blobs();

        let storage = storage_manager.create_storage();
        let result =
            <HashStf as StateTransitionFunction<MockZkvm, MockZkvm, MockDaSpec>>::apply_slot(
                &stf,
                &state_root,
                storage,
                ArrayWitness::default(),
                &block.header,
                relevant_blobs.as_iters(),
                sov_modules_api::ExecutionContext::Node,
            );

        state_root = result.state_root;
        storage_manager.commit(result.change_set);
    }

    let storage = storage_manager.create_storage();
    let root_hash = storage.get_latest_root_hash().unwrap();
    (state_root, root_hash)
}

fn batch(serialized_tx: Vec<u8>) -> Vec<u8> {
    borsh::to_vec(&vec![FullyBakedTx {
        data: serialized_tx,
    }])
    .unwrap()
}
