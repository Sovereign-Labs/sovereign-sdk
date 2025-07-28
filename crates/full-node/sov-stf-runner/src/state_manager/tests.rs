use std::collections::HashMap;
use std::num::NonZero;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use futures::StreamExt;
use proptest::prelude::*;
use rand::SeedableRng;
use sov_db::storage_manager::{NativeChangeSet, NativeStorageManager};
use sov_mock_da::storable::layer::StorableMockDaLayer;
use sov_mock_da::storable::service::StorableMockDaService;
use sov_mock_da::{
    BlockProducingConfig, MockAddress, MockBlock, MockBlockHeader, MockDaConfig, MockDaService,
    MockDaSpec, MockHash, PlannedFork, RandomizationBehaviour, RandomizationConfig,
};
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::provable_height_tracker::InfiniteHeight;
use sov_rollup_interface::common::{HexHash, RollupHeight, SlotNumber};
use sov_rollup_interface::da::{DaSpec, RelevantBlobIters};
use sov_rollup_interface::node::ledger_api::LedgerStateProvider;
use sov_rollup_interface::node::SyncStatus;
use sov_rollup_interface::stf::{
    ApplySlotOutput, BatchReceipt, ExecutionContext, StateTransitionFunction,
};
use sov_rollup_interface::zk::Zkvm;
use sov_state::{
    ArrayWitness, NativeStorage, ProverStorage, SlotKey, SlotValue, StateAccesses, Storage,
};

use super::*;

/// A mock implementation of the [`StateTransitionFunction`]
#[derive(PartialEq, Debug, Clone, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct MockStf;

impl<InnerVm: Zkvm, OuterVm: Zkvm, Da: DaSpec> StateTransitionFunction<InnerVm, OuterVm, Da>
    for MockStf
{
    type Address = Vec<u8>;
    type StateRoot = <ProverStorage<S> as Storage>::Root;
    type GasPrice = ();
    type GenesisParams = ();
    type PreState = ();
    type ChangeSet = ();
    type StorageProof = ();
    type TxReceiptContents = ();
    type BatchReceiptContents = ();
    type Witness = ();

    // Perform one-time initialization for the genesis block.
    fn init_chain(
        &self,
        _genesis_rollup_header: &Da::BlockHeader,
        _base_state: Self::PreState,
        _params: Self::GenesisParams,
    ) -> (Self::StateRoot, ()) {
        (<ProverStorage<S> as Storage>::PRE_GENESIS_ROOT, ())
    }

    fn apply_slot(
        &self,
        _pre_state_root: &Self::StateRoot,
        _base_state: Self::PreState,
        _witness: Self::Witness,
        _slot_header: &Da::BlockHeader,
        _relevant_blobs: RelevantBlobIters<&mut [<Da as DaSpec>::BlobTransaction]>,
        _execution_context: ExecutionContext,
    ) -> ApplySlotOutput<InnerVm, OuterVm, Da, Self> {
        ApplySlotOutput::<InnerVm, OuterVm, Da, Self> {
            state_root: <ProverStorage<S> as Storage>::PRE_GENESIS_ROOT,
            change_set: (),
            proof_receipts: vec![],
            batch_receipts: vec![BatchReceipt {
                batch_hash: [0; 32],
                tx_receipts: vec![],
                ignored_tx_receipts: vec![],
                inner: (),
            }],
            discarded_blobs: Default::default(),
            witness: (),
            rollup_height: RollupHeight::new(0),
        }
    }
}

type Vm = MockZkvm;
type S = sov_state::DefaultStorageSpec<sha2::Sha256>;
type Stf = MockStf;
type StateRoot = <Stf as StateTransitionFunction<Vm, Vm, MockDaSpec>>::StateRoot;
type TestBatchReceiptContents =
    <Stf as StateTransitionFunction<Vm, Vm, MockDaSpec>>::BatchReceiptContents;
type TestTxReceiptContents =
    <Stf as StateTransitionFunction<Vm, Vm, MockDaSpec>>::TxReceiptContents;
type Witness = <Stf as StateTransitionFunction<Vm, Vm, MockDaSpec>>::Witness;
type MockSlotCommit = SlotCommit<MockBlock, Witness, TestTxReceiptContents>;
type TestStateManager<Da> = StateManager<
    StateRoot,
    Witness,
    NativeStorageManager<<Da as DaService>::Spec, ProverStorage<S>>,
    Da,
>;
type TestStateManagerInMemory = TestStateManager<MockDaService>;

const SEQUENCER_ADDRESS: MockAddress = MockAddress::new([0; 32]);
const SEED_1: [u8; 32] = [1; 32];
const SEED_2: [u8; 32] = [2; 32];
const SEED_3: [u8; 32] = [3; 32];

#[tokio::test(flavor = "multi_thread")]
async fn test_empty_state_manager_returns_last_finalized_height() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let mut state_manager = setup_state_manager(tempdir.path()).await?;

    let finality = 1000;
    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);
    da_service.send_transaction(&[10; 10]).await.await??;
    let filtered_block = da_service.get_block_at(1).await?;

    process_continuous_transition(&mut state_manager, filtered_block, &da_service, finality)
        .await?;

    // LedgerDb storage should be updated by that point, so the correct height is returned
    assert_eq!(
        SlotNumber::GENESIS,
        state_manager
            .ledger_db
            .get_latest_finalized_slot_number()
            .await?
    );

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_instant_finality() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let mut state_manager = setup_state_manager(tempdir.path()).await?;

    let (sender, mut receiver) = crate::processes::new_stf_info_channel(
        state_manager.ledger_db.clone(),
        NonZero::new(40).unwrap(),
        NonZero::new(40).unwrap(),
    )
    .await?;
    state_manager.stf_info_sender = Some(sender);
    let da_service = MockDaService::new(SEQUENCER_ADDRESS);

    let mut state_root = *state_manager.get_state_root();
    for height in 1..4 {
        da_service
            .send_transaction(&[height as u8; 10])
            .await
            .await??;
        let filtered_block = da_service.get_block_at(height).await?;
        process_continuous_transition(&mut state_manager, filtered_block.clone(), &da_service, 0)
            .await?;
        // TODO: Check how state manager internal state looks like on instant finality.
        let finalized = receiver.read_next().await?.unwrap();

        if let Some(sender) = state_manager.stf_info_sender.as_ref() {
            sender.inc_next_height_to_receive();
        };

        assert_eq!(height, finalized.slot_number.get());
        assert_eq!(filtered_block.header, finalized.data.da_block_header);
        assert_eq!(state_root, finalized.data.initial_state_root);
        state_root.clone_from(&finalized.data.final_state_root);
        assert_eq!(
            height,
            state_manager
                .ledger_db
                .get_latest_finalized_slot_number()
                .await?
                .get()
        );
    }

    Ok(())
}

// Basic test for single reorg, but detailed check of what state root hash is returned.
#[tokio::test(flavor = "multi_thread")]
async fn test_reorg_happened_correct_block_returned() -> anyhow::Result<()> {
    // The idea of the test is
    // to ensure that the state manager returns the correct block and storage aftera single reorg.
    let tempdir = tempfile::tempdir()?;
    let mut state_manager = setup_state_manager(tempdir.path()).await?;

    let fork_point = 3;
    let fork_happens_at = 6;
    let finality = 5;

    let state_update_receiver = state_manager.state_update_sender.subscribe();

    let mut da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);
    da_service
        .set_planned_fork(PlannedFork::new(
            fork_happens_at,
            fork_point,
            vec![vec![11], vec![22], vec![33], vec![44]],
        ))
        .await?;

    // State root after executing i-th transition
    let mut post_state_roots = Vec::with_capacity(fork_happens_at as usize);
    let mut hash_to_post_state_root: HashMap<MockHash, StateRoot> = HashMap::new();

    for da_height in 1..=fork_happens_at {
        // Not used anywhere, `process_normal_transition` relies on da header to produce changes.
        let blob_data = [da_height as u8; 10];
        da_service.send_transaction(&blob_data).await.await??;
        let filtered_block = da_service.get_block_at(da_height).await?;
        if da_height < fork_happens_at {
            let block_hash = filtered_block.header().hash();
            process_continuous_transition(
                &mut state_manager,
                filtered_block,
                &da_service,
                finality,
            )
            .await?;
            let current_state_root = *state_manager.get_state_root();
            let received_storage = state_update_receiver.borrow().storage.clone();
            let received_storage_root = received_storage.get_latest_root_hash()?;
            assert_eq!(current_state_root, received_storage_root);
            post_state_roots.push(current_state_root);
            hash_to_post_state_root.insert(block_hash, current_state_root);
        } else {
            let (prover_storage, returned_block) = state_manager
                .prepare_storage(filtered_block.clone(), &da_service)
                .await?;
            assert_ne!(filtered_block, returned_block);
            // First non seen block:
            assert_eq!(fork_point + 1, returned_block.header().height());

            assert!(!hash_to_post_state_root.contains_key(&returned_block.header.hash));
            let expected_pre_state_root = hash_to_post_state_root
                .get(&returned_block.header().prev_hash())
                .expect("Should be there");
            assert_eq!(
                expected_pre_state_root,
                state_manager.get_state_root(),
                "Expected (left) state root does not match actual(right) set in StateManager. All state roots: {post_state_roots:?}");

            let returned_storage_root = prover_storage.get_latest_root_hash()?;
            let received_update_info = state_update_receiver.borrow().clone();
            let received_storage_root = received_update_info.storage.get_latest_root_hash()?;
            assert_eq!(returned_storage_root, received_storage_root);
        }
    }
    Ok(())
}

/// This test checks that process_stf_changes goes normally,
/// even when the finalized block progressed above the passed block header.
/// Important invariant, that ledger db receives "true" last finalized height,
/// not height of the last **seen** finalized transition.
/// Basically this test covers the case of "syncing node",
/// and it is an important invariant that LedgerDb gets true finalized height
#[tokio::test(flavor = "multi_thread")]
async fn test_save_last_finalized_larger_than_seen_latest_seen_transition() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let mut state_manager = setup_state_manager(tempdir.path()).await?;
    let finality = 10;
    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);

    let chain_length = 5;
    // Fill some seen transitions without finalizing.
    for height in 1..chain_length {
        da_service
            .send_transaction(&[height as u8; 10])
            .await
            .await??;
        let filtered_block = da_service.get_block_at(height).await?;

        process_continuous_transition(&mut state_manager, filtered_block, &da_service, finality)
            .await?;
        assert_eq!(
            0,
            state_manager
                .ledger_db
                .get_latest_finalized_slot_number()
                .await?
                .get()
        );
    }

    // Here we are going to finalize all things between
    da_service
        .send_transaction(&[chain_length as u8; 10])
        .await
        .await??;

    let filtered_block = da_service.get_block_at(chain_length).await?;
    let (prover_storage, returned_block) = state_manager
        .prepare_storage(filtered_block.clone(), &da_service)
        .await?;

    assert_eq!(filtered_block, returned_block);

    let produce_between = (finality * 3) as u64;
    for _ in 0..produce_between {
        da_service.send_transaction(&[10; 10]).await.await??;
    }

    let last_finalized_height = da_service.get_last_finalized_block_header().await?.height();

    let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
        state_manager.get_state_root().to_owned(),
        prover_storage,
        &da_service,
        filtered_block.clone(),
    )
    .await;

    let slot_commit: MockSlotCommit = SlotCommit::new(filtered_block, Default::default());
    state_manager
        .process_stf_changes(
            &da_service,
            0,
            change_set,
            transition_witness,
            slot_commit,
            Vec::new(),
        )
        .await?;
    check_internal_consistency(&state_manager, finality as usize);

    // Last finalized height written to LedgerDb as it passed.
    assert_eq!(
        last_finalized_height,
        state_manager
            .ledger_db
            .get_latest_finalized_slot_number()
            .await?
            .get()
    );
    Ok(())
}

// Test simulates usage of StateManager by StfRunner
// DaLayer is set up with finality, some empty blocks are padded, and some batches are submitted.
// Then it iterates for `loop_blocks` producing a new block on every loop.
async fn test_progressing_with_shuffle(
    finality: u32,
    empty_padding: u32,
    batches: usize,
    loop_blocks: usize,
    shuffle_after: usize,
    seed: [u8; 32],
) -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let mut state_manager = setup_state_manager(tempdir.path()).await?;

    let da_layer = std::sync::Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(finality).await?,
    ));
    let da_service = StorableMockDaService::new(
        SEQUENCER_ADDRESS,
        da_layer.clone(),
        BlockProducingConfig::OnBatchSubmit {
            block_wait_timeout_ms: Some(3_000),
        },
    )
    .await;
    let mut rng = rand::rngs::SmallRng::from_seed(seed);

    // Empty padding
    da_service
        .produce_n_blocks_now(empty_padding as usize)
        .await?;

    // Blobs
    let blob_data = [10; 10];
    for _ in 0..batches {
        da_service.send_transaction(&blob_data).await.await??;
    }

    if empty_padding == 0 && batches == 0 {
        // Producing height=1, so the main loop can kick in.
        da_service.produce_block_now().await?;
    }

    let mut max_seen_height = 0;
    let mut non_finalized_batches = batches.saturating_sub(finality as usize);
    let mut last_finalized_header = da_service.get_last_finalized_block_header().await?;
    let mut height = match last_finalized_header.height {
        0 => 1,
        h => h,
    };

    let mut seen_transitions: HashMap<MockHash, StateRoot> = HashMap::new();
    let mut finalized_hashes: HashSet<MockHash> = HashSet::new();
    for h in 0..=last_finalized_header.height() {
        finalized_hashes.insert(da_service.get_block_at(h).await?.header().hash());
    }

    // This is a simplified version of `StfRunner
    //  - Track height, adjusts it based on StateManager results
    //  - Produce some changes based on a given block
    //  - Moves on the next height
    for i in 0..loop_blocks {
        // Start with getting block
        let filtered_block = da_service.get_block_at(height).await?;

        let (prover_storage, returned_block) = state_manager
            .prepare_storage(filtered_block, &da_service)
            .await?;

        // Always a new non-seen block
        assert!(
            !seen_transitions.contains_key(&returned_block.header().hash()),
            "Already seen: {}",
            returned_block.header().display()
        );

        let prev_hash = returned_block.header().prev_hash();
        assert!(
            seen_transitions.contains_key(&prev_hash) || finalized_hashes.contains(&prev_hash),
            "prev hash of returned block should be seen or in finalized {} SEEN: {:?} FINALIZED {:?}",
            returned_block.header().display(),
            seen_transitions,
            finalized_hashes,
        );

        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            state_manager.get_state_root().to_owned(),
            prover_storage,
            &da_service,
            returned_block.clone(),
        )
        .await;

        let slot_commit: MockSlotCommit =
            SlotCommit::new(returned_block.clone(), Default::default());

        let state_root_hash = transition_witness.final_state_root;
        state_manager
            .process_stf_changes(
                &da_service,
                0,
                change_set,
                transition_witness,
                slot_commit,
                Vec::new(),
            )
            .await?;
        check_internal_consistency(&state_manager, finality as usize);

        seen_transitions.insert(returned_block.header().hash(), state_root_hash);

        if returned_block.header().height() > max_seen_height {
            max_seen_height = returned_block.header().height();
        }

        height = returned_block.header().height() + 1;

        if let Some(earliest_seen_height) = state_manager.get_earliest_seen_height() {
            assert!(
                earliest_seen_height >= last_finalized_header.height(),
                "older finalized heights are not erased: {} {}: {:?}",
                earliest_seen_height,
                last_finalized_header.height(),
                state_manager.seen_on_height,
            );
            let highest_seen_height = state_manager.seen_on_height.keys().copied().max().unwrap();
            assert!(
                highest_seen_height <= max_seen_height,
                "Inconsistent state transitions, highest seen hight is too large"
            );
        }

        // Check is done, moving the chain forward

        if i > 0 && i % shuffle_after == 0 {
            let mut da_layer = da_layer.write().await;
            da_layer.shuffle_non_finalized_blobs(&mut rng, 0).await?;
        }
        // First, check if we need to submit a blob, so it will keep floating
        // TO
        // last_finalized_header = da_service.get_last_finalized_block_header().await?;

        // New block should always be created with a batch
        if batches >= finality as usize {
            da_service.send_transaction(&blob_data).await.await??;
        } else {
            let next_finalized_block = da_service
                .get_block_at(last_finalized_header.height().saturating_add(1))
                .await?;
            // All batches in next block are going to be finalized, so it won't be possible to shuffle them anymore
            non_finalized_batches =
                non_finalized_batches.saturating_sub(next_finalized_block.batch_blobs.len());
            // We try to maintain number of non finalized batches closer to the original number.
            if non_finalized_batches < batches {
                da_service.send_transaction(&blob_data).await.await??;
                non_finalized_batches += 1;
            } else {
                da_service.produce_block_now().await?;
            }
        }
        last_finalized_header = da_service.get_last_finalized_block_header().await?;
        finalized_hashes.insert(last_finalized_header.hash());
    }
    Ok(())
}

// This test check that a chain always returns the non-executed block, even if chain forks are restored.
// We emulate the return of the chain by having only a single blob "floating" between a number of empty blocks.
// Empty blocks have the same root hash, so we can check that we don't execute empty blocks several times.
#[tokio::test(flavor = "multi_thread")]
async fn test_double_reorg_chain_restored() -> anyhow::Result<()> {
    let finality = 20;
    let empty_blocks_padding = 15;
    let batches = 1;
    let loop_blocks = 100;
    for seed in [SEED_1, SEED_2, SEED_3] {
        test_progressing_with_shuffle(
            finality,
            empty_blocks_padding,
            batches,
            loop_blocks,
            3,
            seed,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shuffle_with_multiple_blobs() -> anyhow::Result<()> {
    let finality = 20;
    let empty_blocks_padding = 0;
    let batches = 5;
    let loop_blocks = 50;
    for seed in [SEED_1, SEED_2, SEED_3] {
        test_progressing_with_shuffle(
            finality,
            empty_blocks_padding,
            batches,
            loop_blocks,
            2,
            seed,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_shuffle_with_deeper_reorgs() -> anyhow::Result<()> {
    let finality = 20;
    let empty_blocks_padding = 10;
    let batches = 5;
    let loop_blocks = 50;
    for seed in [SEED_1, SEED_2, SEED_3] {
        test_progressing_with_shuffle(
            finality,
            empty_blocks_padding,
            batches,
            loop_blocks,
            10,
            seed,
        )
        .await?;
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn test_with_frequent_periodic_batch_production() -> anyhow::Result<()> {
    // sov_test_utils::initialize_logging();
    let tempdir = tempfile::tempdir()?;
    let mut state_manager = setup_state_manager(tempdir.path()).await?;

    let finality = 50;
    let (sender, mut receiver) = tokio::sync::watch::channel(());
    receiver.mark_unchanged();

    let da_service = StorableMockDaService::from_config(
        MockDaConfig {
            connection_string: "sqlite::memory:".to_string(),
            sender_address: SEQUENCER_ADDRESS,
            finalization_blocks: finality,
            block_producing: BlockProducingConfig::Periodic { block_time_ms: 100 },
            da_layer: None,
            randomization: Some(RandomizationConfig {
                seed: HexHash::from(SEED_1),
                // At every new block
                reorg_interval: 1..2,
                behaviour: RandomizationBehaviour::only_shuffle(0),
            }),
        },
        receiver,
    )
    .await;
    {
        let spammer = da_service.clone();
        let _handle: tokio::task::JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            let mut finalized_blocks = spammer.subscribe_finalized_header().await?;
            let blob = vec![10, 10];
            while let Some(res) = finalized_blocks.next().await {
                let _ = match res {
                    Ok(b) => b,
                    Err(_err) => {
                        break;
                    }
                };
                spammer.send_transaction(&blob).await.await??;
            }
            Ok(())
        });
    }

    let mut height = match da_service.get_last_finalized_block_header().await?.height() {
        0 => 1,
        h => h,
    };
    let final_height = 100;

    let mut seen_transitions: HashMap<MockHash, StateRoot> = HashMap::new();

    while height < final_height {
        let filtered_block = da_service.get_block_at(height).await?;
        let (prover_storage, returned_block) = state_manager
            .prepare_storage(filtered_block, &da_service)
            .await?;

        assert!(
            !seen_transitions.contains_key(&returned_block.header().hash()),
            "Already seen: {}",
            returned_block.header().display()
        );
        // TODO: Check prev_hash connected to something already seen.

        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            state_manager.get_state_root().to_owned(),
            prover_storage,
            &da_service,
            returned_block.clone(),
        )
        .await;

        let slot_commit: MockSlotCommit =
            SlotCommit::new(returned_block.clone(), Default::default());

        let state_root_hash = transition_witness.final_state_root;
        state_manager
            .process_stf_changes(
                &da_service,
                0,
                change_set,
                transition_witness,
                slot_commit,
                Vec::new(),
            )
            .await?;
        check_internal_consistency(&state_manager, finality as usize);
        seen_transitions.insert(returned_block.header().hash(), state_root_hash);

        height = returned_block.header().height() + 1;
    }

    sender.send(())?;
    Ok(())
}

// After each "prepare_storage" there are (empty_blobs + batch_blobs) number of blocks produced.
// `shuffle_after` controls how often shuffle happens, based on number of blocks last shuffle happened
async fn test_chain_progress_between_prepare_storage_and_save_changes(
    finality: u32,
    // Total number of iterations.
    loop_blocks: usize,
    // Progression parameters.
    empty_blobs: usize,
    batch_blobs: usize,
    shuffle_after: u64,
    seed: [u8; 32],
) -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let mut state_manager = setup_state_manager(tempdir.path()).await?;

    let mut rng = rand::rngs::SmallRng::from_seed(seed);

    let da_layer = std::sync::Arc::new(tokio::sync::RwLock::new(
        StorableMockDaLayer::new_in_memory(finality).await?,
    ));
    let da_service = StorableMockDaService::new(
        SEQUENCER_ADDRESS,
        da_layer.clone(),
        BlockProducingConfig::OnBatchSubmit {
            block_wait_timeout_ms: Some(3_000),
        },
    )
    .await;
    // To kick start things.
    da_service.produce_block_now().await?;

    let mut seen_transitions: HashMap<MockHash, StateRoot> = HashMap::new();
    let mut height = 1;
    let mut last_shuffled_height = 0;

    for _ in 0..loop_blocks {
        let filtered_block = da_service.get_block_at(height).await?;
        let (prover_storage, returned_block) = state_manager
            .prepare_storage(filtered_block, &da_service)
            .await?;

        assert!(
            !seen_transitions.contains_key(&returned_block.header().hash()),
            "Already seen: {}",
            returned_block.header().display()
        );
        // TODO: Check prev_hash connected to something already seen.

        // Here we do some progression of the chain
        {
            da_service.produce_n_blocks_now(empty_blobs).await?;
            for i in 0..batch_blobs {
                let blob_data = [i as u8, i as u8];
                da_service.send_transaction(&blob_data).await.await??;
            }
            let head = da_service.get_head_block_header().await?;
            let head_height = head.height().saturating_sub(last_shuffled_height);
            if head_height > shuffle_after {
                let mut da_layer = da_layer.write().await;
                da_layer.shuffle_non_finalized_blobs(&mut rng, 0).await?;
                last_shuffled_height = head_height;
            }
        }

        // Then saving
        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            state_manager.get_state_root().to_owned(),
            prover_storage,
            &da_service,
            returned_block.clone(),
        )
        .await;

        let slot_commit: MockSlotCommit =
            SlotCommit::new(returned_block.clone(), Default::default());

        let state_root_hash = transition_witness.final_state_root;
        state_manager
            .process_stf_changes(
                &da_service,
                0,
                change_set,
                transition_witness,
                slot_commit,
                Vec::new(),
            )
            .await?;
        check_internal_consistency(&state_manager, finality as usize);

        seen_transitions.insert(returned_block.header().hash(), state_root_hash);

        height = returned_block.header().height() + 1;
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_chain_progress_between_prepare_and_save_instant_finality() -> anyhow::Result<()> {
    for seed in [SEED_1, SEED_2, SEED_3] {
        // With empty blobs
        test_chain_progress_between_prepare_storage_and_save_changes(0, 60, 3, 3, 6, seed).await?;
        // Without empty blobs
        test_chain_progress_between_prepare_storage_and_save_changes(0, 60, 0, 3, 6, seed).await?;
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_chain_progress_between_prepare_and_save_non_instant_finality() -> anyhow::Result<()> {
    let finality = 5;

    for seed in [SEED_1, SEED_2, SEED_3] {
        // With empty blobs
        test_chain_progress_between_prepare_storage_and_save_changes(finality, 60, 1, 2, 6, seed)
            .await?;
        // Shuffle every time
        test_chain_progress_between_prepare_storage_and_save_changes(finality, 60, 1, 2, 3, seed)
            .await?;
        // Without empty blobs
        test_chain_progress_between_prepare_storage_and_save_changes(finality, 60, 0, 3, 6, seed)
            .await?;
    }

    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn proptest_shuffling_with_different_params(
        finality in prop_oneof![
            Just(0u32),
            Just(1u32),
            Just(5u32)
        ],
        loop_blocks in 1..=20usize,
        batches in prop_oneof![
            Just(0usize),
            Just(2usize),
            Just(5usize)
        ],
        reshuffle_after in prop_oneof![
            Just(1usize),
            Just(3usize),
            Just(5usize)
        ],
        seed in prop_oneof![
            Just(SEED_1),
            Just(SEED_2),
            Just(SEED_3),
        ]
        ) {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on( async {
                    let test_future = test_progressing_with_shuffle(
                        finality,
                        0,
                        batches,
                        loop_blocks,
                        reshuffle_after,
                        seed,
                    );
                    tokio::time::timeout(std::time::Duration::from_secs(5), test_future).await.unwrap().unwrap();
            });
        }

    #[test]
    fn proptest_chain_prorgress_between(
        finality in prop_oneof![
            Just(0u32),
            Just(1u32),
            Just(5u32)
        ],
        loop_blocks in 1..=20usize,
        batches in prop_oneof![
            Just(1usize),
            Just(2usize),
            Just(5usize)
        ],
        reshuffle_after in prop_oneof![
            Just(1u64),
            Just(3u64),
            Just(5u64)
        ],
        seed in prop_oneof![
            Just(SEED_1),
            Just(SEED_2),
            Just(SEED_3),
        ]
        ) {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on( async {
                    let test_future = test_chain_progress_between_prepare_storage_and_save_changes(
                        finality,
                        loop_blocks,
                        0,
                        batches,
                        reshuffle_after,
                        seed,
                    );
                    tokio::time::timeout(std::time::Duration::from_secs(5), test_future).await.unwrap().unwrap();
            });
        }
}

// Fail case tests
/// Normal changes tracked in state manager, some of them finalized.
/// Then new [`MockDaService`] is initialized and new blocks are submitted, so new different header is finalized.
/// This way we can have a case where [`StateManager`] cannot backtrack to continuous transition,
/// because finalized were eliminated. This behaviour is similar as starting from a non-finalized block and then whole chain switches.
#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "Finalized header changed")]
async fn test_change_in_finalized_header() {
    let tempdir = tempfile::tempdir().unwrap();
    let mut state_manager = setup_state_manager(tempdir.path()).await.unwrap();

    let chain_length = 5;
    let finality = 3;

    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);

    for height in 1..=chain_length {
        da_service
            .send_transaction(&[height as u8; 10])
            .await
            .await
            .unwrap()
            .unwrap();
        let filtered_block = da_service.get_block_at(height).await.unwrap();
        process_continuous_transition(
            &mut state_manager,
            filtered_block.clone(),
            &da_service,
            finality,
        )
        .await
        .unwrap();
    }

    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);
    for height in 1..=chain_length {
        da_service
            .send_transaction(&[(height * 10) as u8; 10])
            .await
            .await
            .unwrap()
            .unwrap();
    }

    let alien_block = da_service
        .get_block_at(da_service.get_head_block_header().await.unwrap().height())
        .await
        .unwrap();

    state_manager
        .prepare_storage(alien_block, &da_service)
        .await
        .unwrap();
}

// On empty internal state, state manager should check if passed block is finalized
// And return last finalized.
#[tokio::test(flavor = "multi_thread")]
async fn test_state_manager_starts_from_non_finalized_height() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let mut state_manager = setup_state_manager(tempdir.path()).await?;

    let chain_length = 7;
    let finality = 5;

    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);
    for height in 1..=chain_length {
        da_service
            .send_transaction(&[(height * 10) as u8; 10])
            .await
            .await??;
    }

    let last_finalized_header = da_service.get_last_finalized_block_header().await?;
    // Should be allowed, because storage has continuous data
    let next_to_finalized = da_service
        .get_block_at(last_finalized_header.height() + 1)
        .await?;
    // Should not be allowed
    let not_next_to_finalized = da_service
        .get_block_at(last_finalized_header.height() + 2)
        .await?;

    let (_prover_storage, returned_block_1) = state_manager
        .prepare_storage(next_to_finalized.clone(), &da_service)
        .await?;

    assert_eq!(returned_block_1, next_to_finalized);

    let (_prover_storage, returned_block_2) = state_manager
        .prepare_storage(not_next_to_finalized.clone(), &da_service)
        .await?;

    assert_ne!(returned_block_2, not_next_to_finalized);
    assert_eq!(returned_block_2, next_to_finalized);

    Ok(())
}

// TODO: Add tests that verification of finalized transitions only contains finalized blocks

// TODO: Test state manager starts from non finalized height, then chain forks and all transitions are obliterated.
// prepare_storage will panic probably
// But process storage might just eliminate all transitions and it will start from finalized height.
// Is it bad? Probably yes, because

// ----------------
// Helper functions
async fn setup_storage_manager(
    path: &std::path::Path,
) -> anyhow::Result<(
    StateRoot,
    NativeStorageManager<MockDaSpec, ProverStorage<S>>,
)> {
    let mut storage_manager: NativeStorageManager<MockDaSpec, ProverStorage<S>> =
        NativeStorageManager::new(path)?;
    let genesis_block = MockBlock::default_at_height(0);
    let genesis_header = genesis_block.header().clone();
    let (genesis_storage, ledger_state) = storage_manager.create_state_for(&genesis_header)?;
    let ledger_db = LedgerDb::with_reader(ledger_state)?;

    let (state_root, change_set) = produce_synthetic_changes::<MockDaSpec>(
        genesis_storage,
        &genesis_header,
        <ProverStorage<S> as Storage>::PRE_GENESIS_ROOT,
    );

    let data_to_commit: SlotCommit<_, TestBatchReceiptContents, TestTxReceiptContents> =
        SlotCommit::new(genesis_block, Default::default());
    let mut ledger_change_set = ledger_db.materialize_slot(data_to_commit, state_root.as_ref())?;
    let finalized_slot_changes =
        ledger_db.materialize_latest_finalize_slot(SlotNumber::GENESIS, SlotNumber::GENESIS)?;
    ledger_change_set.merge(finalized_slot_changes);

    storage_manager.save_change_set(&genesis_header, change_set, ledger_change_set)?;
    storage_manager.finalize(&genesis_header)?;

    Ok((state_root, storage_manager))
}

async fn setup_state_manager<Da>(path: &std::path::Path) -> anyhow::Result<TestStateManager<Da>>
where
    Da: DaService<Error = anyhow::Error, Spec = MockDaSpec>,
{
    let (state_root, mut storage_manager) = setup_storage_manager(path).await?;
    let genesis_header = MockBlockHeader::from_height(0);
    let (stf_state, ledger_state) = storage_manager.create_state_after(&genesis_header)?;
    let ledger_db = LedgerDb::with_reader(ledger_state)?;
    let update_info = query_state_update_info(&ledger_db, stf_state).await?;

    // Update channel, receiver does not need to be alive
    let (state_update_sender, _state_update_recv) = watch::channel(update_info);

    let (sync_status_sender, _rec) = tokio::sync::watch::channel(SyncStatus::START);

    let sync_state = Arc::new(DaSyncState {
        synced_da_height: AtomicU64::new(0),
        target_da_height: AtomicU64::new(u64::MAX),
        sync_status_sender,
    });

    let mut state_manager = StateManager::new(
        storage_manager,
        ledger_db,
        state_root,
        state_update_sender,
        None,
        Box::new(InfiniteHeight),
        sync_state,
        std::time::Duration::from_millis(10),
    )?;
    state_manager.startup().await?;

    Ok(state_manager)
}

// Writes to user space concatenation of block height bytes and block hash
fn produce_synthetic_changes<Da: DaSpec>(
    prover_storage: ProverStorage<S>,
    block_header: &Da::BlockHeader,
    pre_state_root: <ProverStorage<S> as Storage>::Root,
) -> (<ProverStorage<S> as Storage>::Root, NativeChangeSet) {
    let mut data = block_header.height().to_le_bytes().to_vec();
    data.extend_from_slice(block_header.hash().as_ref());
    let mut accesses = StateAccesses::default();
    accesses
        .user
        .ordered_writes
        .push((SlotKey::from(data.clone()), Some(SlotValue::from(data))));
    let (state_root, state_update) = prover_storage
        .compute_state_update(accesses, &ArrayWitness::default(), pre_state_root)
        .unwrap();
    let change_set = prover_storage.materialize_changes(state_update);

    (state_root, change_set)
}

async fn produce_synthetic_state_transition_witness<Da: DaService>(
    initial_state_root: <ProverStorage<S> as Storage>::Root,
    prover_storage: ProverStorage<S>,
    da_service: &Da,
    filtered_block: Da::FilteredBlock,
) -> (
    NativeChangeSet,
    StateTransitionWitness<StateRoot, Witness, Da::Spec>,
) {
    let (state_root, change_set) = produce_synthetic_changes::<Da::Spec>(
        prover_storage,
        filtered_block.header(),
        initial_state_root,
    );
    let (relevant_blobs, relevant_proofs) = da_service
        .extract_relevant_blobs_with_proof(&filtered_block)
        .await;

    let transition_witness = StateTransitionWitness {
        initial_state_root,
        final_state_root: state_root,
        da_block_header: filtered_block.header().clone(),
        relevant_proofs,
        relevant_blobs,
        witness: (),
    };

    (change_set, transition_witness)
}

// Passed `filtered_block` supposed to be a continuation of the current chain,
// So this helper function performs transition and checks that there is no error
async fn process_continuous_transition(
    state_manager: &mut TestStateManagerInMemory,
    filtered_block: MockBlock,
    da_service: &MockDaService,
    finality: u32,
) -> anyhow::Result<()> {
    let (prover_storage, returned_block) = state_manager
        .prepare_storage(filtered_block.clone(), da_service)
        .await?;

    assert_eq!(filtered_block, returned_block);

    let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
        state_manager.get_state_root().to_owned(),
        prover_storage,
        da_service,
        filtered_block.clone(),
    )
    .await;

    let slot_commit: MockSlotCommit = SlotCommit::new(filtered_block, Default::default());
    state_manager
        .process_stf_changes(
            da_service,
            0,
            change_set,
            transition_witness,
            slot_commit,
            Vec::new(),
        )
        .await?;
    check_internal_consistency(state_manager, finality as usize);

    Ok(())
}

fn check_internal_consistency<Da>(state_manager: &TestStateManager<Da>, finality: usize)
where
    Da: DaService<Error = anyhow::Error>,
{
    // Ensure consistency between seen_on_height and state_on_block
    for (height, seen_blocks) in &state_manager.seen_on_height {
        assert!(
            !seen_blocks.is_empty(),
            "empty seen blocks at height: {height}. Dirty!"
        );
        for seen_hash in seen_blocks {
            assert_eq!(
                state_manager
                    .state_on_block
                    .get(seen_hash)
                    .map(|state| state.block_header.hash())
                    .as_ref(),
                Some(seen_hash)
            );
            if let Some(state) = state_manager.state_on_block.get(seen_hash) {
                assert_eq!(
                    height, &state.block_header.height(),
                    "Inconsistency found: height in seen_on_height ({}) does not match state_on_block ({})",
                    height, state.block_header.height()
                );
                assert_eq!(
                    &state.block_header.prev_hash(),
                    &state_manager.get_prev_hash(seen_hash),
                    "Inconsistency found: prev_hash in seen_on_height ({}) does not match state_on_block ({})",
                    height, state.block_header.prev_hash()
                );
            } else {
                panic!("Block {seen_hash} from seen_on_height is missing in state_on_block");
            }
        }
    }

    // Check if all blocks in state_on_block are present in seen_on_height
    for (block_hash, state) in &state_manager.state_on_block {
        let block_header = &state.block_header;
        assert_eq!(&block_header.hash(), block_hash);
        if !state_manager
            .seen_on_height
            .get(&block_header.height())
            .expect("Block is missing from seen_on_height")
            .iter()
            .any(|seen_hash| seen_hash == block_hash)
        {
            panic!(
                "Block {} from state_on_block is missing in seen_on_height",
                block_header.display(),
            );
        }
    }

    // We should not observe more hights than there are non-finalized blocks possible.
    let seen_on_height_size = state_manager.seen_on_height.len();
    assert!(
        seen_on_height_size <= finality,
        "Size of seen_on_height={seen_on_height_size} is more than finality={finality}"
    );

    let earliest_seen_height = state_manager.get_earliest_seen_height();
    let highest_seen_height = state_manager.get_highest_seen_height();

    // There should be no gaps between heights of observed blocks.
    let expected_continuous_size = match (earliest_seen_height, highest_seen_height) {
        (Some(earliest), Some(latest)) => latest
            .saturating_sub(earliest)
            .checked_add(1)
            .expect("bug in test") as usize,
        (None, None) => 0,
        _ => panic!("Impossible, both values derived from same map"),
    };

    assert_eq!(seen_on_height_size, expected_continuous_size);
}
