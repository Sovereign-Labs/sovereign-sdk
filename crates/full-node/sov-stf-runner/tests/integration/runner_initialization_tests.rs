use std::sync::Arc;

use sov_mock_da::{MockAddress, MockBlock, MockDaService};
use sov_mock_zkvm::MockZkvm;
use sov_rollup_interface::node::da::DaService;

use crate::helpers::hash_stf::HashStf;
use crate::helpers::runner_init::{initialize_runner, InitVariant};
type MockInitVariant = InitVariant<HashStf, MockZkvm, MockZkvm, MockDaService>;

#[tokio::test(flavor = "multi_thread")]
async fn init_and_restart() {
    init_and_restart_inner().await;
}

#[should_panic]
#[tokio::test]
async fn runner_initialization_fails_if_tokio_runtime_is_not_multi_threaded() {
    // It doesn't really matter what we do here as long as it involves
    // initializing a runner. So we might as well reuse the logic of another
    // test.
    init_and_restart_inner().await;
}

async fn init_and_restart_inner() {
    let genesis_block = MockBlock {
        header: Default::default(),
        batch_blobs: vec![],
        proof_blobs: vec![],
    };
    let init_variant: MockInitVariant = InitVariant::Genesis {
        block: genesis_block,
        genesis_params: vec![1],
    };

    let tmpdir = tempfile::tempdir().unwrap();
    let path = tmpdir.path();

    let da_service = Arc::new(MockDaService::new(MockAddress::new([11u8; 32])));

    let state_root_after_genesis = {
        let (runner, node) =
            initialize_runner(da_service.clone(), path, init_variant, 1, None).await;
        let state_root = *runner.get_state_root();
        node.stop().await;
        state_root
    };

    let init_variant_2 = InitVariant::Initialized {
        prev_state_root: state_root_after_genesis,
        last_finalized_block_header: da_service.get_last_finalized_block_header().await.unwrap(),
    };

    let state_root_2 = {
        let (runner_2, node_2) = initialize_runner(da_service, path, init_variant_2, 1, None).await;
        let state_root = *runner_2.get_state_root();
        node_2.stop().await;
        state_root
    };

    assert_eq!(state_root_after_genesis, state_root_2);
}
