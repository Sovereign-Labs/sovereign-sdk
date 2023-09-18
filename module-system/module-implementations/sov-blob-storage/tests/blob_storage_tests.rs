use sov_blob_storage::BlobStorage;
use sov_chain_state::{ChainState, ChainStateConfig};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Module, WorkingSet};
use sov_rollup_interface::mocks::{MockAddress, MockBlob, MockDaSpec};
use sov_state::ProverStorage;

type C = DefaultContext;
type B = MockBlob;
type Da = MockDaSpec;

#[test]
fn empty_test() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

    let chain_state = ChainState::<C, Da>::default();
    let initial_slot_height = 1;
    let chain_state_config = ChainStateConfig {
        initial_slot_height,
        current_time: Default::default(),
    };
    chain_state
        .genesis(&chain_state_config, &mut working_set)
        .unwrap();

    let blob_storage = BlobStorage::<C, Da>::default();

    let blobs: Vec<B> =
        blob_storage.take_blobs_for_slot_height(initial_slot_height, &mut working_set);

    assert!(blobs.is_empty());
}

#[test]
fn store_and_retrieve_standard() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

    let chain_state = ChainState::<C, Da>::default();
    let initial_slot_height = 1;
    let chain_state_config = ChainStateConfig {
        initial_slot_height,
        current_time: Default::default(),
    };
    chain_state
        .genesis(&chain_state_config, &mut working_set)
        .unwrap();

    let blob_storage = BlobStorage::<C, Da>::default();

    assert!(blob_storage
        .take_blobs_for_slot_height(1, &mut working_set)
        .is_empty());
    assert!(blob_storage
        .take_blobs_for_slot_height(2, &mut working_set)
        .is_empty());
    assert!(blob_storage
        .take_blobs_for_slot_height(3, &mut working_set)
        .is_empty());
    assert!(blob_storage
        .take_blobs_for_slot_height(4, &mut working_set)
        .is_empty());

    let sender = MockAddress::from([1u8; 32]);
    let dummy_hash = [2u8; 32];

    let blob_1 = B::new(vec![1, 2, 3], sender, dummy_hash);
    let blob_2 = B::new(vec![3, 4, 5], sender, dummy_hash);
    let blob_3 = B::new(vec![6, 7, 8], sender, dummy_hash);
    let blob_4 = B::new(vec![9, 9, 9], sender, dummy_hash);
    let blob_5 = B::new(vec![0, 1, 0], sender, dummy_hash);

    let slot_2_blobs = vec![blob_1, blob_2, blob_3];
    let slot_2_blob_refs: Vec<&MockBlob> = slot_2_blobs.iter().collect();
    let slot_3_blobs = vec![blob_4];
    let slot_3_blob_refs: Vec<&MockBlob> = slot_3_blobs.iter().collect();
    let slot_4_blobs = vec![blob_5];
    let slot_4_blob_refs: Vec<&MockBlob> = slot_4_blobs.iter().collect();

    blob_storage
        .store_blobs(2, &slot_2_blob_refs, &mut working_set)
        .unwrap();
    blob_storage
        .store_blobs(3, &slot_3_blob_refs, &mut working_set)
        .unwrap();
    blob_storage
        .store_blobs(4, &slot_4_blob_refs, &mut working_set)
        .unwrap();

    assert_eq!(
        slot_2_blobs,
        blob_storage.take_blobs_for_slot_height(2, &mut working_set)
    );
    assert!(blob_storage
        .take_blobs_for_slot_height(2, &mut working_set)
        .is_empty());

    assert_eq!(
        slot_3_blobs,
        blob_storage.take_blobs_for_slot_height(3, &mut working_set)
    );
    assert!(blob_storage
        .take_blobs_for_slot_height(3, &mut working_set)
        .is_empty());

    assert_eq!(
        slot_4_blobs,
        blob_storage.take_blobs_for_slot_height(4, &mut working_set)
    );
    assert!(blob_storage
        .take_blobs_for_slot_height(4, &mut working_set)
        .is_empty());
}
