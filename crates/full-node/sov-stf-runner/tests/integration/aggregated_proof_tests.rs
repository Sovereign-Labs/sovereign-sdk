use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use sov_mock_da::{MockAddress, MockBlock, MockBlockHeader, MockDaService, MockDaSpec};
use sov_mock_zkvm::{MockCodeCommitment, MockZkVerifier};
use sov_rollup_interface::zk::aggregated_proof::{
    AggregateProofVerifier, AggregatedProofPublicData, SerializedAggregatedProof,
};
use sov_state::StorageRoot;
use tokio::task::JoinHandle;

use crate::helpers::hash_stf::S;
use crate::helpers::runner_init::{initialize_runner, InitVariant, TestNode};

#[tokio::test(flavor = "multi_thread")]
async fn fetch_aggregated_proof_test_sync() -> anyhow::Result<()> {
    let test_case = TestCase::new(5);
    run_make_proof_sync(test_case, 3).await?;

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn fetch_aggregated_proof_test_async() -> anyhow::Result<()> {
    let test_case = TestCase::new(5);
    tokio::time::timeout(
        std::time::Duration::from_secs(60),
        run_make_proof_async(test_case, 3),
    )
    .await??;

    Ok(())
}

// In this test, proofs are created just after batch is submitted to the DA.
async fn run_make_proof_sync(test_case: TestCase, nb_of_threads: usize) -> anyhow::Result<()> {
    let tmpdir = tempfile::tempdir()?;
    let jump = test_case.jump();

    let nb_of_batches = test_case.input.nb_of_batches;
    let (mut test_node, runner_task) = spawn(jump, nb_of_threads, tmpdir.path()).await;

    for batch_number in 0..nb_of_batches {
        test_node.send_transaction().await?;
        // Update the visible rollup height
        test_node.make_block_proof();

        if (batch_number + 1) % jump == 0 {
            test_node.wait_for_aggregated_proof_posted_to_da().await?;
        }
    }

    test_node.try_send_aggregated_proof().await?;
    test_node.make_block_proof();

    let mut init_slot = 1;

    let mut pub_data = None;
    for _ in (0..nb_of_batches).step_by(jump) {
        init_slot = calculate_and_check_rollup_height(init_slot, jump);

        let resp = test_node.wait_for_aggregated_proof_saved_in_db().await;
        pub_data = Some(verify_aggregated_proof(resp.proof)?);
    }

    test_case.assert(&pub_data.unwrap());
    test_node.stop().await;
    // Joining runner task to avoid error:
    // pthread lock: Invalid argument
    //
    // that is probably coming from rocksdb de-allocation.
    // Or this hides some awful nasty bug somewhere deep in 3rd party library.
    // Runner suppose to fail, as it won't receive new blocks
    let _x = runner_task.await?;

    Ok(())
}

fn verify_aggregated_proof(
    agg_proof: SerializedAggregatedProof,
) -> anyhow::Result<AggregatedProofPublicData<MockAddress, MockDaSpec, StorageRoot<S>>> {
    let verifier = AggregateProofVerifier::<MockZkVerifier>::new(MockCodeCommitment::default());
    verifier.verify(&agg_proof)
}

// In this test, proofs are created after multiple batches are submitted to the DA.
async fn run_make_proof_async(test_case: TestCase, nb_of_threads: usize) -> anyhow::Result<()> {
    let tmpdir = tempfile::tempdir()?;
    let jump = test_case.jump();
    let nb_of_batches: u64 = test_case.input.nb_of_batches as u64;
    let provable_height_ref = Arc::new(AtomicU64::new(0));
    let (mut test_node, runner_task) = spawn(test_case.jump(), nb_of_threads, tmpdir.path()).await;

    for _ in 0..nb_of_batches {
        // Update the provable height
        test_node.send_transaction().await?;
        provable_height_ref.fetch_add(1, Ordering::SeqCst);
    }

    for _ in 0..nb_of_batches {
        test_node.make_block_proof();
    }

    for _ in (0..nb_of_batches).step_by(jump) {
        test_node.wait_for_aggregated_proof_posted_to_da().await?;
    }

    test_node.try_send_aggregated_proof().await?;
    test_node.make_block_proof();

    let mut init_slot = 1;

    let mut pub_data = None;
    for _ in (0..nb_of_batches).step_by(jump) {
        init_slot = calculate_and_check_rollup_height(init_slot, jump);

        let resp = test_node.wait_for_aggregated_proof_saved_in_db().await;
        pub_data = Some(verify_aggregated_proof(resp.proof)?);
    }

    test_case.assert(&pub_data.unwrap());
    test_node.stop().await;
    let _x = runner_task.await?;

    Ok(())
}

fn calculate_and_check_rollup_height(init_slot: u64, jump: usize) -> u64 {
    let final_slot = init_slot + jump as u64 - 1;
    final_slot + 1
}

async fn spawn(
    jump: usize,
    nb_of_threads: usize,
    path: impl AsRef<std::path::Path>,
) -> (TestNode, JoinHandle<anyhow::Result<()>>) {
    let genesis_block = MockBlock {
        header: MockBlockHeader::from_height(0),
        batch_blobs: vec![],
        proof_blobs: vec![],
    };
    let init_variant = InitVariant::Genesis {
        block: genesis_block,
        genesis_params: vec![1],
    };

    let da_service =
        Arc::new(MockDaService::new(MockAddress::new([11u8; 32])).with_wait_attempts(200));

    let (mut runner, test_node) = initialize_runner(
        da_service,
        path.as_ref(),
        init_variant,
        jump,
        Some(nb_of_threads),
    )
    .await;

    let join_handle = tokio::spawn(async move {
        runner.run_in_process().await.map_err(|error| {
            tracing::warn!(?error, "Runner returned a error during execution");
            error
        })
    });

    (test_node, join_handle)
}

#[derive(Clone, Copy)]
struct Input {
    jump: usize,
    nb_of_batches: usize,
}

#[derive(Clone, Copy)]
struct Output {
    initial_slot_number: u64,
}

#[derive(Clone, Copy)]
struct TestCase {
    input: Input,
    output: Output,
}

impl TestCase {
    fn new(jump: usize) -> Self {
        // Generate 7 aggregate-proofs worth of blocks
        let nb_of_batches = 7 * jump;
        // The initial rollup height of the final proof.
        // The first proof covers slots 1..=jump, the second jump+1..=(2*jump), etc.
        let initial_slot_number = (6 * jump + 1) as u64;
        Self {
            input: Input {
                jump,
                nb_of_batches,
            },
            output: Output {
                initial_slot_number,
            },
        }
    }

    fn jump(&self) -> usize {
        self.input.jump
    }

    fn final_slot_number(&self) -> u64 {
        self.output.initial_slot_number + (self.input.jump as u64) - 1
    }

    fn assert(
        &self,
        public_data: &AggregatedProofPublicData<MockAddress, MockDaSpec, StorageRoot<S>>,
    ) {
        assert_eq!(
            self.output.initial_slot_number,
            public_data.initial_slot_number.get(),
        );

        assert_eq!(
            self.final_slot_number(),
            public_data.final_slot_number.get()
        );
    }
}
