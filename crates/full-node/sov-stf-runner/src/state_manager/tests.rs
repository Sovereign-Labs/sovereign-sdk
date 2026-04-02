use std::collections::HashMap;
use std::num::NonZero;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use futures::StreamExt;
use proptest::prelude::*;
use rand::{Rng, SeedableRng};
use serde::Deserialize;
use sov_db::storage_manager::{NativeChangeSet, NativeStorageManager};
use sov_mock_da::storable::layer::{Randomizer, StorableMockDaLayer};
use sov_mock_da::storable::StorableMockDaService;
use sov_mock_da::{
    BlockProducingConfig, MockAddress, MockBlock, MockBlockHeader, MockDaConfig, MockDaService,
    MockDaSpec, MockHash, PlannedFork, RandomizationBehaviour, RandomizationConfig,
};
use sov_modules_api::provable_height_tracker::InfiniteHeight;
use sov_rollup_interface::common::{HexHash, RollupHeight, SlotNumber};
use sov_rollup_interface::da::{DaSpec, RelevantBlobIters};
use sov_rollup_interface::node::ledger_api::LedgerStateProvider;
use sov_rollup_interface::node::SyncStatus;
use sov_rollup_interface::stf::GenesisParams;
use sov_rollup_interface::stf::{
    ApplySlotOutput, BatchReceipt, ExecutionContext, StateTransitionFunction,
};
use sov_state::{
    ArrayWitness, NativeStorage, ProverStorage, SlotKey, SlotValue, StateAccesses, Storage,
};

use super::*;
// We need a proof receipt type whose first and last generics are serializable, and middle two params are daspec and state root.
// This is never constructed - just used to satisfy the type checker.
type DummyProofReceipt = PartialProofReceipt<u64, MockDaSpec, StateRoot, u64>;

const DA_POLLING_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

/// Helper to extract (pre_state, pre_state_root, ledger_pre_state) from BlockCandidateResolution::KnownContinuation.
/// Panics if result is NoMatch - use this only in tests that expect continuation.
fn unwrap_continuation<S, R, L>(result: BlockCandidateResolution<S, R, L>) -> (S, R, L) {
    match result {
        BlockCandidateResolution::KnownContinuation {
            pre_state,
            pre_state_root,
            ledger_pre_state,
        } => (pre_state, pre_state_root, ledger_pre_state),
        BlockCandidateResolution::NoMatch { height_to_fetch } => {
            panic!("Expected KnownContinuation, got NoMatch(height_to_fetch={height_to_fetch})")
        }
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct MockGenesisParams;

impl GenesisParams for MockGenesisParams {
    fn genesis_slot_number(&self) -> u64 {
        0
    }
}

/// A mock implementation of the [`StateTransitionFunction`]
#[derive(PartialEq, Debug, Clone, Eq, serde::Serialize, serde::Deserialize, Default)]
pub struct MockStf;

impl<Da: DaSpec> StateTransitionFunction<Da> for MockStf {
    type StateRoot = <ProverStorage<S> as Storage>::Root;
    type Address = Vec<u8>;
    type GenesisParams = MockGenesisParams;
    type PreState = ();
    type ChangeSet = ();
    type GasPrice = ();
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
    ) -> ApplySlotOutput<Da, Self> {
        ApplySlotOutput::<Da, Self> {
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

type S = sov_state::DefaultStorageSpec<sha2::Sha256>;
type Stf = MockStf;
type StateRoot = <Stf as StateTransitionFunction<MockDaSpec>>::StateRoot;
type TestBatchReceiptContents = <Stf as StateTransitionFunction<MockDaSpec>>::BatchReceiptContents;
type TestTxReceiptContents = <Stf as StateTransitionFunction<MockDaSpec>>::TxReceiptContents;
type Witness = <Stf as StateTransitionFunction<MockDaSpec>>::Witness;
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
    let finality = 1000;
    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);

    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    for h in 1..=10 {
        da_service.send_transaction(&[10; 10]).await.await??;
        let filtered_block = da_service.get_block_at(h).await?;

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
    }

    shutdown_sender.send(())?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn test_instant_finality() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let da_service = MockDaService::new(SEQUENCER_ADDRESS);
    let (mut state_manager, initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    let (sender, mut receiver) = crate::processes::new_stf_info_channel(
        state_manager.ledger_db.clone(),
        NonZero::new(40).unwrap(),
        NonZero::new(40).unwrap(),
    )
    .await?;
    state_manager.stf_info_sender = Some(sender);

    let mut state_root = initial_state_root;
    for height in 1..4 {
        da_service
            .send_transaction(&[height as u8; 10])
            .await
            .await??;
        let filtered_block = da_service.get_block_at(height).await?;
        // Sleep here more, to ensure that latest finalized header has been pulled.
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;
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
        let ledger_last_finalized_height = state_manager
            .ledger_db
            .get_latest_finalized_slot_number()
            .await?
            .get();
        let diff = height
            .checked_sub(ledger_last_finalized_height)
            .expect("Ledger cannot see future finalized height");
        assert!(
            diff <= 2,
            "Ledger cannot lag behind last finalized height by more than 2"
        );
    }

    shutdown_sender.send(())?;

    Ok(())
}

// Basic test for single reorg, but detailed check of what state root hash is returned.
#[tokio::test(flavor = "multi_thread")]
async fn test_reorg_happened_correct_block_returned() -> anyhow::Result<()> {
    // The idea of the test is
    // to ensure that the state manager returns the correct block and storage after a single reorg.
    let tempdir = tempfile::tempdir()?;

    let fork_point = 3;
    let fork_happens_at = 6;
    let finality = 5;
    let mut da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);
    da_service
        .set_planned_fork(PlannedFork::new(
            fork_happens_at,
            fork_point,
            vec![vec![11], vec![22], vec![33], vec![44]],
        ))
        .await?;

    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    let state_update_receiver = state_manager.state_update_sender.subscribe();

    // State root after executing i-th transition
    let mut post_state_roots = Vec::with_capacity(fork_happens_at as usize);
    let mut hash_to_post_state_root: HashMap<MockHash, StateRoot> = HashMap::new();

    for da_height in 1..=fork_happens_at {
        // Not used anywhere, `process_normal_transition` relies on da header to produce changes.
        let blob_data = [da_height as u8; 10];
        da_service.send_transaction(&blob_data).await.await??;
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;
        let filtered_block = da_service.get_block_at(da_height).await?;
        if da_height < fork_happens_at {
            let block_hash = filtered_block.header().hash();
            let current_state_root = process_continuous_transition(
                &mut state_manager,
                filtered_block,
                &da_service,
                finality,
            )
            .await?;
            let received_storage = state_update_receiver.borrow().storage.clone();
            let received_storage_root = received_storage.get_latest_root_hash()?;
            assert_eq!(current_state_root, received_storage_root);
            post_state_roots.push(current_state_root);
            hash_to_post_state_root.insert(block_hash, current_state_root);
        } else {
            // Reorg detected - is_good_continuation should return NoMatch with fork point height
            let resolution = state_manager
                .check_continuation(filtered_block.header(), &da_service)
                .await?;
            let height_to_fetch = match resolution {
                BlockCandidateResolution::NoMatch { height_to_fetch } => height_to_fetch,
                BlockCandidateResolution::KnownContinuation { .. } => {
                    panic!("Expected NoMatch for reorg, got KnownContinuation")
                }
            };
            // First non seen block should be at fork_point + 1
            assert_eq!(fork_point + 1, height_to_fetch);

            // Now fetch and process the fork point block
            let fork_block = da_service.get_block_at(height_to_fetch).await?;
            let (prover_storage, pre_state_root, ledger_pre_state) = unwrap_continuation(
                state_manager
                    .check_continuation(fork_block.header(), &da_service)
                    .await?,
            );
            check_internal_consistency(&state_manager, finality as usize);

            assert!(!hash_to_post_state_root.contains_key(&fork_block.header().hash()));
            let expected_pre_state_root = hash_to_post_state_root
                .get(&fork_block.header().prev_hash())
                .expect("Should be there");
            assert_eq!(
                expected_pre_state_root,
                &pre_state_root,
                "Expected (left) state root does not match actual(right) from KnownContinuation. All state roots: {post_state_roots:?}");

            // State update is not called during re-org detection. So we process transition first
            let _returned_storage_prev_root = prover_storage.get_latest_root_hash()?;
            // TODO: Should we check this prev_root against something

            let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
                pre_state_root,
                prover_storage,
                &da_service,
                fork_block.clone(),
            )
            .await;

            let final_state_root = transition_witness.final_state_root;
            let slot_commit: MockSlotCommit = SlotCommit::new(fork_block, Default::default());
            state_manager
                .process_stf_changes(
                    change_set,
                    ledger_pre_state,
                    transition_witness,
                    slot_commit,
                    Vec::new(),
                    Vec::<DummyProofReceipt>::new(),
                )
                .await?;
            check_internal_consistency(&state_manager, finality as usize);

            let received_update_info = state_update_receiver.borrow().clone();
            let received_storage_root = received_update_info.storage.get_latest_root_hash()?;
            assert_eq!(final_state_root, received_storage_root);
        }
    }

    shutdown_sender.send(())?;

    Ok(())
}

/// This test checks that process_stf_changes goes normally,
/// even when the finalized block progressed above the passed block header.
/// Important invariant, ledger db receives the last **processed finalized rollup transition**
/// and not **the last seen  finalized DA height**
/// Basically this test covers the case of "syncing node",
/// and it is an important invariant that LedgerDb gets finalized height that it has processed
#[tokio::test(flavor = "multi_thread")]
async fn test_save_last_finalized_larger_than_seen_latest_seen_transition() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let finality = 10;
    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);
    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

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
    let (prover_storage, pre_state_root, ledger_pre_state) = unwrap_continuation(
        state_manager
            .check_continuation(filtered_block.header(), &da_service)
            .await?,
    );

    let produce_between = (finality * 3) as u64;
    for _ in 0..produce_between {
        da_service.send_transaction(&[10; 10]).await.await??;
    }

    let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
        pre_state_root,
        prover_storage,
        &da_service,
        filtered_block.clone(),
    )
    .await;

    let slot_commit: MockSlotCommit = SlotCommit::new(filtered_block, Default::default());
    tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;
    state_manager
        .process_stf_changes(
            change_set,
            ledger_pre_state,
            transition_witness,
            slot_commit,
            Vec::new(),
            Vec::<DummyProofReceipt>::new(),
        )
        .await?;
    check_internal_consistency(&state_manager, finality as usize);

    // The last finalized height is not written to LedgerDb directly,
    // only the last processed finalized height.
    assert_eq!(
        chain_length,
        state_manager
            .ledger_db
            .get_latest_finalized_slot_number()
            .await?
            .get()
    );
    shutdown_sender.send(())?;
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
    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

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
    // Always start from height 1 (adjacent to genesis) since StateManager
    // is initialized with last_processed_finalized_header at genesis (height 0)
    let mut height = 1;

    let mut seen_transitions: HashMap<MockHash, StateRoot> = HashMap::new();
    let mut finalized_hashes: HashSet<MockHash> = HashSet::new();
    // Only genesis is finalized from StateManager's perspective at startup
    finalized_hashes.insert(da_service.get_block_at(0).await?.header().hash());

    // This is a simplified version of `StfRunner
    //  - Track height, adjusts it based on StateManager results
    //  - Produce some changes based on a given block
    //  - Moves on the next height
    for i in 0..loop_blocks {
        // Start with getting block - always start from height 1 (adjacent to genesis)
        // since StateManager's last_processed_finalized_header is at genesis
        let initial_block = da_service.get_block_at(height).await?;

        // Keep trying until we get a continuation (handles reorgs)
        let (prover_storage, pre_state_root, ledger_pre_state, returned_block) =
            resolve_to_continuation(&mut state_manager, &da_service, initial_block).await?;

        // Validate block chain integrity
        validate_block_chain_integrity(&returned_block, &seen_transitions, &finalized_hashes);

        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            pre_state_root,
            prover_storage,
            &da_service,
            returned_block.clone(),
        )
        .await;

        let slot_commit: MockSlotCommit =
            SlotCommit::new(returned_block.clone(), Default::default());

        let state_root_hash = transition_witness.final_state_root;
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;
        state_manager
            .process_stf_changes(
                change_set,
                ledger_pre_state,
                transition_witness,
                slot_commit,
                Vec::new(),
                Vec::<DummyProofReceipt>::new(),
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

    shutdown_sender.send(())?;
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
            failure_behavior: Default::default(),
        },
        receiver,
    )
    .await;

    let (mut state_manager, _initial_state_root, shutdown_sender_2) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

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
    let mut finalized_hashes: HashSet<MockHash> = HashSet::new();
    // Genesis is finalized at startup
    finalized_hashes.insert(da_service.get_block_at(0).await?.header().hash());

    while height < final_height {
        let initial_block = da_service.get_block_at(height).await?;

        // Keep trying until we get a continuation (handles reorgs)
        let (prover_storage, pre_state_root, ledger_pre_state, returned_block) =
            resolve_to_continuation(&mut state_manager, &da_service, initial_block).await?;

        // Validate block chain integrity
        validate_block_chain_integrity(&returned_block, &seen_transitions, &finalized_hashes);

        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            pre_state_root,
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
                change_set,
                ledger_pre_state,
                transition_witness,
                slot_commit,
                Vec::new(),
                Vec::<DummyProofReceipt>::new(),
            )
            .await?;
        check_internal_consistency(&state_manager, finality as usize);
        seen_transitions.insert(returned_block.header().hash(), state_root_hash);

        // Track finalized headers
        let last_finalized = da_service.get_last_finalized_block_header().await?;
        finalized_hashes.insert(last_finalized.hash());

        height = returned_block.header().height() + 1;
    }

    shutdown_sender_2.send(())?;
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

    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    // To kick-start things.
    da_service.produce_block_now().await?;

    let mut seen_transitions: HashMap<MockHash, StateRoot> = HashMap::new();
    let mut finalized_hashes: HashSet<MockHash> = HashSet::new();
    // Genesis is finalized at startup
    finalized_hashes.insert(da_service.get_block_at(0).await?.header().hash());

    let mut height = 1;
    let mut last_shuffled_height = 0;

    for _ in 0..loop_blocks {
        let initial_block = da_service.get_block_at(height).await?;

        // Keep trying until we get a continuation (handles reorgs)
        let (prover_storage, pre_state_root, ledger_pre_state, returned_block) =
            resolve_to_continuation(&mut state_manager, &da_service, initial_block).await?;

        // Validate blockchain integrity
        validate_block_chain_integrity(&returned_block, &seen_transitions, &finalized_hashes);

        // Produce more blocks (simulates chain advancing while we process)
        da_service.produce_n_blocks_now(empty_blobs).await?;
        for i in 0..batch_blobs {
            let blob_data = [i as u8, i as u8];
            da_service.send_transaction(&blob_data).await.await??;
        }

        // Process the state transition
        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            pre_state_root,
            prover_storage,
            &da_service,
            returned_block.clone(),
        )
        .await;

        let slot_commit: MockSlotCommit =
            SlotCommit::new(returned_block.clone(), Default::default());

        let state_root_hash = transition_witness.final_state_root;
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;
        state_manager
            .process_stf_changes(
                change_set,
                ledger_pre_state,
                transition_witness,
                slot_commit,
                Vec::new(),
                Vec::<DummyProofReceipt>::new(),
            )
            .await?;
        check_internal_consistency(&state_manager, finality as usize);

        seen_transitions.insert(returned_block.header().hash(), state_root_hash);

        // Track finalized headers
        let last_finalized = da_service.get_last_finalized_block_header().await?;
        finalized_hashes.insert(last_finalized.hash());

        // Check if we should rewind (AFTER processing, so it affects the next iteration)
        let head = da_service.get_head_block_header().await?;
        let head_height = head.height();
        let last_finalized_height = last_finalized.height();
        let blocks_since_last_rewind = head_height.saturating_sub(last_shuffled_height);

        // Only rewind if we have blocks to rewind to (between finalized and head)
        // and we're past the current block we just processed
        let processed_height = returned_block.header().height();
        let safe_min = std::cmp::max(last_finalized_height, processed_height);

        if blocks_since_last_rewind > shuffle_after && head_height > safe_min + 1 {
            let mut da_layer = da_layer.write().await;
            // Rewind to a random height, ensuring at least one block above processed
            let height_to_rewind = rng.gen_range((safe_min + 1)..head_height);
            da_layer.rewind_to_height(height_to_rewind as u32).await?;
            last_shuffled_height = height_to_rewind;
        }

        // Next height is always the one after what we just processed
        height = returned_block.header().height() + 1;
    }

    shutdown_sender.send(())?;

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
        test_chain_progress_between_prepare_storage_and_save_changes(finality, 100, 1, 2, 6, seed)
            .await?;
        // Shuffle every time
        test_chain_progress_between_prepare_storage_and_save_changes(finality, 100, 1, 2, 3, seed)
            .await?;
        // Without empty blobs
        test_chain_progress_between_prepare_storage_and_save_changes(finality, 100, 0, 3, 6, seed)
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
        loop_blocks in 1..=30usize,
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
                    tokio::time::timeout(std::time::Duration::from_secs(10), test_future).await.unwrap().unwrap();
            });
        }
}

// Fail case tests
/// Normal changes tracked in state manager, some of them finalized.
/// Then new [`MockDaService`] is initialized and new blocks are submitted, so new different header is finalized.
/// This way we can have a case where [`StateManager`] cannot backtrack to continuous transition,
/// because finalized were eliminated. This behaviour is similar as starting from a non-finalized block and then whole chain switches.
#[tokio::test(flavor = "multi_thread")]
#[should_panic(expected = "Finalized chain inconsistency detected")]
async fn test_change_in_finalized_header() {
    let tempdir = tempfile::tempdir().unwrap();

    let chain_length = 5;
    let finality = 3;

    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);

    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone())
            .await
            .unwrap();

    for height in 1..=chain_length {
        da_service
            .send_transaction(&[height as u8; 10])
            .await
            .await
            .unwrap()
            .unwrap();
        let filtered_block = da_service.get_block_at(height).await.unwrap();
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;
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

    // An alien block from a different DA chain should return NoMatch
    let _result = state_manager
        .check_continuation(alien_block.header(), &da_service)
        .await
        .unwrap();

    shutdown_sender.send(()).unwrap();
}

// On empty internal state, if we pass a block that is not adjacent to
// last_processed_finalized_header (genesis), the state manager should return
// NoMatch with a height to fetch (recovered via fork point search).
#[tokio::test(flavor = "multi_thread")]
async fn test_state_manager_recovers_from_non_adjacent_block() {
    let tempdir = tempfile::tempdir().unwrap();
    let chain_length = 7;
    let finality = 5;

    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);

    let (mut state_manager, _initial_state_root, _shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone())
            .await
            .unwrap();

    for height in 1..=chain_length {
        da_service
            .send_transaction(&[(height * 10) as u8; 10])
            .await
            .await
            .unwrap()
            .unwrap();
    }

    let last_finalized_header = da_service.get_last_finalized_block_header().await.unwrap();
    // This is NOT adjacent to last_processed_finalized_header (genesis),
    // so state manager should return NoMatch to guide us to the right block.
    let non_adjacent_block = da_service
        .get_block_at(last_finalized_header.height() + 1)
        .await
        .unwrap();

    // Should return NoMatch with height 1 (the first block after genesis)
    let result = state_manager
        .check_continuation(non_adjacent_block.header(), &da_service)
        .await
        .unwrap();

    match result {
        BlockCandidateResolution::NoMatch { height_to_fetch } => {
            // Fork point search should find block 1 as the first unprocessed block
            // whose predecessor (genesis) we've seen (it's the finalized header)
            assert_eq!(height_to_fetch, 1, "Should guide us to block 1");
        }
        BlockCandidateResolution::KnownContinuation { .. } => {
            panic!("Should not be a continuation - block is not adjacent to genesis");
        }
    }
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

async fn setup_state_manager<Da>(
    storage_path: &std::path::Path,
    da_service: Da,
) -> anyhow::Result<(
    TestStateManager<Da>,
    StateRoot,
    tokio::sync::watch::Sender<()>,
)>
where
    Da: DaService<Error = anyhow::Error, Spec = MockDaSpec>,
{
    let (initial_state_root, mut storage_manager) = setup_storage_manager(storage_path).await?;
    let genesis_height = 0;
    let genesis_header = MockBlockHeader::from_height(genesis_height);
    let (stf_state, ledger_state) = storage_manager.create_state_after(&genesis_header)?;
    let ledger_db = LedgerDb::with_reader(ledger_state)?;

    let (sync_status_sender, _rec) = tokio::sync::watch::channel(SyncStatus::START);

    let sync_state = Arc::new(DaSyncState {
        synced_da_height: AtomicU64::new(genesis_height),
        target_da_height: AtomicU64::new(u64::MAX),
        sync_status_sender,
    });
    let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(());
    shutdown_rx.mark_unchanged();

    let update_info = query_state_update_info(&ledger_db, stf_state, sync_state.as_ref()).await?;
    // Update channel, receiver does not need to be alive
    let (state_update_sender, _state_update_recv) = watch::channel(update_info);

    let da_header_provider = DaServiceWithCachedFinalizedHeaders::new(
        Arc::new(da_service),
        shutdown_rx,
        DA_POLLING_INTERVAL,
    )
    .await?;

    let mut state_manager = StateManager::new(
        storage_manager,
        ledger_db,
        initial_state_root,
        state_update_sender,
        None,
        Box::new(InfiniteHeight),
        sync_state,
        da_header_provider,
        genesis_height,
        genesis_header,
    );
    state_manager.startup().await?;

    Ok((state_manager, initial_state_root, shutdown_tx))
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
        .push((SlotKey::from_slice(&data), Some(SlotValue::from(data))));
    let (state_root, state_update) = prover_storage
        .compute_state_update(accesses, &ArrayWitness::default(), pre_state_root, None)
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
// So this helper function performs transition and checks that there is no error.
// Returns the final state root after the transition.
async fn process_continuous_transition(
    state_manager: &mut TestStateManagerInMemory,
    filtered_block: MockBlock,
    da_service: &MockDaService,
    finality: u32,
) -> anyhow::Result<StateRoot> {
    let (prover_storage, pre_state_root, ledger_pre_state) = unwrap_continuation(
        state_manager
            .check_continuation(filtered_block.header(), da_service)
            .await?,
    );

    let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
        pre_state_root,
        prover_storage,
        da_service,
        filtered_block.clone(),
    )
    .await;

    let final_state_root = transition_witness.final_state_root;
    let slot_commit: MockSlotCommit = SlotCommit::new(filtered_block, Default::default());
    state_manager
        .process_stf_changes(
            change_set,
            ledger_pre_state,
            transition_witness,
            slot_commit,
            Vec::new(),
            Vec::<DummyProofReceipt>::new(),
        )
        .await?;
    check_internal_consistency(state_manager, finality as usize);

    Ok(final_state_root)
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

    // We should not observe more heights than there are non-finalized blocks possible.
    // With instant finality (finality=0), we still have 1 block being processed before cleanup,
    // so we allow finality + 1 as the upper bound.
    let seen_on_height_size = state_manager.seen_on_height.len();
    let max_allowed = finality.saturating_add(1);
    assert!(
        seen_on_height_size <= max_allowed,
        "Size of seen_on_height={seen_on_height_size} is more than max_allowed={max_allowed} (finality={finality})"
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

/// Resolves a block to a valid continuation, handling reorgs by fetching suggested blocks.
/// Returns (pre_state, pre_state_root, ledger_pre_state, final_block).
///
/// This helper handles the common pattern of:
/// 1. Call check_continuation
/// 2. If NoMatch, fetch the suggested block and retry
/// 3. Continue until we get KnownContinuation
async fn resolve_to_continuation<Da>(
    state_manager: &mut TestStateManager<Da>,
    da_service: &Da,
    initial_block: MockBlock,
) -> anyhow::Result<(
    ProverStorage<S>,
    StateRoot,
    <NativeStorageManager<MockDaSpec, ProverStorage<S>> as HierarchicalStorageManager<
        MockDaSpec,
    >>::LedgerState,
    MockBlock,
)>
where
    Da: DaService<Error = anyhow::Error, Spec = MockDaSpec, FilteredBlock = MockBlock>,
{
    let mut block = initial_block;
    loop {
        match state_manager
            .check_continuation(block.header(), da_service)
            .await?
        {
            BlockCandidateResolution::KnownContinuation {
                pre_state,
                pre_state_root,
                ledger_pre_state,
            } => return Ok((pre_state, pre_state_root, ledger_pre_state, block)),
            BlockCandidateResolution::NoMatch { height_to_fetch } => {
                block = da_service.get_block_at(height_to_fetch).await?;
            }
        }
    }
}

/// Validate that a returned block hasn't been seen and its parent is known.
/// Panics with descriptive message if validation fails.
fn validate_block_chain_integrity(
    block: &MockBlock,
    seen_transitions: &HashMap<MockHash, StateRoot>,
    finalized_hashes: &std::collections::HashSet<MockHash>,
) {
    let hash = block.header().hash();
    let prev_hash = block.header().prev_hash();

    assert!(
        !seen_transitions.contains_key(&hash),
        "Block already seen: {}",
        block.header().display()
    );

    assert!(
        seen_transitions.contains_key(&prev_hash) || finalized_hashes.contains(&prev_hash),
        "Block's parent not known: {} (prev_hash={}). SEEN: {:?} FINALIZED: {:?}",
        block.header().display(),
        prev_hash,
        seen_transitions.keys().collect::<Vec<_>>(),
        finalized_hashes
    );
}

/// Verify LedgerDb consistency after processing a block.
/// Checks:
/// 1. Head slot matches expected slot number
/// 2. State root matches expected value
/// 3. Next slot number is head + 1
/// 4. Finalized height is monotonically increasing (if previous value provided)
async fn check_ledger_consistency(
    ledger_db: &LedgerDb,
    expected_state_root: &[u8],
    expected_slot_number: SlotNumber,
    prev_finalized: Option<SlotNumber>,
) {
    // 1. Verify head slot matches expected
    let (actual_slot, stored_slot) = ledger_db
        .get_head_slot()
        .expect("should get head slot")
        .expect("head slot should exist");
    assert_eq!(
        actual_slot, expected_slot_number,
        "slot number mismatch: expected {expected_slot_number:?}, got {actual_slot:?}",
    );
    assert_eq!(
        stored_slot.state_root.as_ref(),
        expected_state_root,
        "state root mismatch at slot {expected_slot_number:?}",
    );

    // 2. Verify next slot number is head + 1
    let next_items = ledger_db
        .get_next_items_numbers()
        .expect("should get next items");
    assert_eq!(
        next_items.slot_number,
        expected_slot_number.next(),
        "next slot number mismatch: expected {:?}, got {:?}",
        expected_slot_number.next(),
        next_items.slot_number
    );

    // 3. Verify finalized height is monotonic (if we have previous)
    if let Some(prev) = prev_finalized {
        let current_finalized = ledger_db
            .get_latest_finalized_slot_number()
            .await
            .expect("should get finalized");
        assert!(
            current_finalized >= prev,
            "finalized height decreased: {prev:?} -> {current_finalized:?}",
        );
    }
}

/// Tests StateManager behavior when DA layer reports stale headers (below actual finalized height).
/// Uses the same pattern as `test_progressing_with_shuffle` since StorableMockDaService is required for Randomizer.
#[tokio::test(flavor = "multi_thread")]
async fn test_rewind_below_finalized_instant_finality() -> anyhow::Result<()> {
    // Reuse existing test infrastructure with RewindBelowLastFinalized behavior
    test_progressing_with_rewind_below_finalized(0, 5, 5, 15, SEED_1).await
}

async fn test_progressing_with_rewind_below_finalized(
    finality: u32,
    batches: usize,
    max_depth: u32,
    loop_blocks: usize,
    seed: [u8; 32],
) -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
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
    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    // Submit blobs
    let blob_data = [10; 10];
    for _ in 0..batches {
        da_service.send_transaction(&blob_data).await.await??;
    }

    // Enable RewindBelowLastFinalized from the start
    {
        let mut layer = da_layer.write().await;
        layer.set_randomizer(Randomizer::from_config(RandomizationConfig {
            seed: HexHash::new(seed),
            reorg_interval: 1..2, // Always trigger
            behaviour: RandomizationBehaviour::RewindBelowLastFinalized { max_depth },
        }));
    }

    let mut height = 1u64;
    for _ in 0..loop_blocks {
        let initial_block = da_service.get_block_at(height).await?;

        // Keep trying until we get a continuation (handles reorgs)
        let (prover_storage, pre_state_root, ledger_pre_state, returned_block) =
            resolve_to_continuation(&mut state_manager, &da_service, initial_block).await?;

        // Note: This test exercises abnormal DA behavior (RewindBelowLastFinalized),
        // so we skip validate_block_chain_integrity as finalized headers may be stale.

        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            pre_state_root,
            prover_storage,
            &da_service,
            returned_block.clone(),
        )
        .await;

        let slot_commit: MockSlotCommit =
            SlotCommit::new(returned_block.clone(), Default::default());

        state_manager
            .process_stf_changes(
                change_set,
                ledger_pre_state,
                transition_witness,
                slot_commit,
                Vec::new(),
                Vec::<DummyProofReceipt>::new(),
            )
            .await?;
        // Skip check_internal_consistency - this test exercises abnormal DA behavior
        // where finalized headers may be stale, so normal consistency rules don't apply

        height = returned_block.header().height() + 1;

        // Keep submitting blobs to advance chain
        da_service.send_transaction(&blob_data).await.await??;
    }

    shutdown_sender.send(())?;
    Ok(())
}

// ==================== DA Failure Handling Tests ====================

/// Test that StateManager handles DA errors gracefully during fork point search.
/// Uses the new FailureBehavior::FailAfterNCalls to inject failures.
#[tokio::test(flavor = "multi_thread")]
async fn test_binary_search_handles_da_error() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let finality = 5;

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

    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    // Process several blocks normally first to build up state
    let blob_data = [10; 10];
    for _ in 0..8 {
        da_service.send_transaction(&blob_data).await.await??;
    }

    // Process first few blocks to populate state_on_block
    for height in 1..=5u64 {
        let filtered_block = da_service.get_block_at(height).await?;
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;

        let (prover_storage, pre_state_root, ledger_pre_state, block) =
            resolve_to_continuation(&mut state_manager, &da_service, filtered_block).await?;
        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            pre_state_root,
            prover_storage,
            &da_service,
            block.clone(),
        )
        .await;
        let slot_commit: MockSlotCommit = SlotCommit::new(block, Default::default());
        state_manager
            .process_stf_changes(
                change_set,
                ledger_pre_state,
                transition_witness,
                slot_commit,
                Vec::new(),
                Vec::<DummyProofReceipt>::new(),
            )
            .await?;
    }

    // Now trigger a reorg by shuffling - this will cause binary search on next check_continuation
    {
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);
        let mut da_layer = da_layer.write().await;
        da_layer.shuffle_non_finalized_blobs(&mut rng, 0).await?;
    }

    // Configure DA to fail after 2 successful get_block_header_at calls
    // This will cause an error during the binary search
    da_service.set_fail_after_n_calls(2).await;

    // Try to get continuation - binary search should fail due to DA error
    let new_block = da_service.get_block_at(6).await?;
    let result = state_manager
        .check_continuation(new_block.header(), &da_service)
        .await;

    // The error should propagate cleanly
    let err = match result {
        Err(e) => e,
        Ok(_) => panic!("Expected error from DA failure, got Ok"),
    };
    let err_msg = err.to_string();
    assert!(
        err_msg.contains("Injected failure"),
        "Error should mention injected failure: {err_msg}",
    );

    // Clear failure behavior and verify we can recover
    da_service.clear_failure_behavior().await;

    // Now the operation should succeed
    let result = state_manager
        .check_continuation(new_block.header(), &da_service)
        .await;
    match result {
        Ok(_) => {}
        Err(e) => panic!("Should succeed after clearing failure, got: {e}"),
    }

    shutdown_sender.send(())?;
    Ok(())
}

/// Test that StateManager handles reorgs that occur during the binary search.
/// Uses FailureBehavior::ReorgDuringCall to trigger a shuffle mid-search.
#[tokio::test(flavor = "multi_thread")]
async fn test_reorg_during_binary_search() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let finality = 10;

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

    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    // Build up a chain with multiple blocks
    let blob_data = [10; 10];
    for _ in 0..15 {
        da_service.send_transaction(&blob_data).await.await??;
    }

    // Process several blocks to populate state
    for height in 1..=10u64 {
        let filtered_block = da_service.get_block_at(height).await?;
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;

        let (prover_storage, pre_state_root, ledger_pre_state, block) =
            resolve_to_continuation(&mut state_manager, &da_service, filtered_block).await?;
        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            pre_state_root,
            prover_storage,
            &da_service,
            block.clone(),
        )
        .await;
        let slot_commit: MockSlotCommit = SlotCommit::new(block, Default::default());
        state_manager
            .process_stf_changes(
                change_set,
                ledger_pre_state,
                transition_witness,
                slot_commit,
                Vec::new(),
                Vec::<DummyProofReceipt>::new(),
            )
            .await?;
    }

    // Trigger initial reorg
    {
        let mut rng = rand::rngs::StdRng::seed_from_u64(100);
        let mut da_layer = da_layer.write().await;
        da_layer.shuffle_non_finalized_blobs(&mut rng, 0).await?;
    }

    // Configure DA to trigger another reorg when height 7 is queried
    // This simulates a reorg happening mid-binary-search
    da_service.set_reorg_during_get_block(7).await;

    // Get a block that will trigger binary search
    let new_block = da_service.get_block_at(11).await?;

    // The check_continuation should handle the mid-search reorg gracefully
    // by detecting the head change and retrying
    let seen_transitions: HashMap<MockHash, StateRoot> = HashMap::new();
    let mut finalized_hashes: HashSet<MockHash> = HashSet::new();
    finalized_hashes.insert(da_service.get_block_at(0).await?.header().hash());

    // Use resolve_to_continuation which handles retries
    let (prover_storage, pre_state_root, ledger_pre_state, returned_block) =
        resolve_to_continuation(&mut state_manager, &da_service, new_block).await?;

    // We should get a valid continuation even after the mid-search reorg
    assert!(
        !seen_transitions.contains_key(&returned_block.header().hash()),
        "Should return unseen block"
    );

    // Process the returned block to verify it's valid
    let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
        pre_state_root,
        prover_storage,
        &da_service,
        returned_block.clone(),
    )
    .await;

    let slot_commit: MockSlotCommit = SlotCommit::new(returned_block, Default::default());
    state_manager
        .process_stf_changes(
            change_set,
            ledger_pre_state,
            transition_witness,
            slot_commit,
            Vec::new(),
            Vec::<DummyProofReceipt>::new(),
        )
        .await?;

    // Internal consistency should still hold
    check_internal_consistency(&state_manager, finality as usize);

    shutdown_sender.send(())?;
    Ok(())
}

// ==================== LedgerDb Consistency Tests ====================

/// Test that LedgerDb remains consistent after processing blocks.
/// Verifies state roots, slot numbers, and finalized height monotonicity.
#[tokio::test(flavor = "multi_thread")]
async fn test_ledger_consistency_after_processing() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let finality = 3;
    let da_service = MockDaService::new(SEQUENCER_ADDRESS).with_finality(finality);

    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    let mut prev_finalized: Option<SlotNumber> = None;

    for height in 1..=10u64 {
        da_service
            .send_transaction(&[height as u8; 10])
            .await
            .await??;
        let filtered_block = da_service.get_block_at(height).await?;
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;

        let state_root = process_continuous_transition(
            &mut state_manager,
            filtered_block,
            &da_service,
            finality,
        )
        .await?;

        // Verify LedgerDb consistency - state_root is what we just committed
        let expected_slot = SlotNumber::new(height);
        check_ledger_consistency(
            &state_manager.ledger_db,
            state_root.as_ref(),
            expected_slot,
            prev_finalized,
        )
        .await;

        // Track previous finalized for monotonicity check
        prev_finalized = Some(
            state_manager
                .ledger_db
                .get_latest_finalized_slot_number()
                .await?,
        );
    }

    shutdown_sender.send(())?;
    Ok(())
}

/// Test that finalized height is strictly monotonic (never decreases).
#[tokio::test(flavor = "multi_thread")]
async fn test_finalized_height_monotonic() -> anyhow::Result<()> {
    let tempdir = tempfile::tempdir()?;
    let finality = 5;

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

    let (mut state_manager, _initial_state_root, shutdown_sender) =
        setup_state_manager(tempdir.path(), da_service.clone()).await?;

    let blob_data = [10; 10];
    let mut prev_finalized = state_manager
        .ledger_db
        .get_latest_finalized_slot_number()
        .await?;

    let mut rng = rand::rngs::StdRng::seed_from_u64(12345);

    // Process many blocks with periodic shuffles to stress test finalization
    for i in 0..30 {
        da_service.send_transaction(&blob_data).await.await??;
        let height = (i + 1) as u64;

        let initial_block = da_service.get_block_at(height).await?;
        let (prover_storage, pre_state_root, ledger_pre_state, returned_block) =
            resolve_to_continuation(&mut state_manager, &da_service, initial_block).await?;

        let (change_set, transition_witness) = produce_synthetic_state_transition_witness(
            pre_state_root,
            prover_storage,
            &da_service,
            returned_block.clone(),
        )
        .await;

        let slot_commit: MockSlotCommit = SlotCommit::new(returned_block, Default::default());
        tokio::time::sleep(DA_POLLING_INTERVAL * 2).await;
        state_manager
            .process_stf_changes(
                change_set,
                ledger_pre_state,
                transition_witness,
                slot_commit,
                Vec::new(),
                Vec::<DummyProofReceipt>::new(),
            )
            .await?;

        // Check that finalized height never decreases
        let current_finalized = state_manager
            .ledger_db
            .get_latest_finalized_slot_number()
            .await?;
        assert!(
            current_finalized >= prev_finalized,
            "Finalized height decreased from {prev_finalized:?} to {current_finalized:?} at iteration {i}",
        );
        prev_finalized = current_finalized;

        // Occasionally trigger shuffles to test consistency under reorgs
        if i > 0 && i % 5 == 0 {
            let mut da_layer = da_layer.write().await;
            da_layer.shuffle_non_finalized_blobs(&mut rng, 0).await?;
        }
    }

    shutdown_sender.send(())?;
    Ok(())
}
