use sov_blob_storage::BlobStorage;
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::Genesis;
use sov_state::{ProverStorage, WorkingSet};

type C = DefaultContext;

#[test]
fn empty_test() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let blob_storage = BlobStorage::<C>::default();

    blob_storage.genesis(&(), &mut working_set).unwrap();

    let blobs = blob_storage.take_blobs_for_block_number(1, &mut working_set);

    assert!(blobs.is_empty());
}

#[test]
fn store_and_retrieve_standard() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let blob_storage = BlobStorage::<C>::default();

    blob_storage.genesis(&(), &mut working_set).unwrap();

    assert!(blob_storage
        .take_blobs_for_block_number(1, &mut working_set)
        .is_empty());
    assert!(blob_storage
        .take_blobs_for_block_number(2, &mut working_set)
        .is_empty());
    assert!(blob_storage
        .take_blobs_for_block_number(3, &mut working_set)
        .is_empty());
    assert!(blob_storage
        .take_blobs_for_block_number(4, &mut working_set)
        .is_empty());

    let blob_1 = vec![1, 2, 3];
    let blob_2 = vec![3, 4, 5];
    let blob_3 = vec![6, 7, 8];
    let blob_4 = vec![9, 9, 9];
    let blob_5 = vec![0, 1, 0];

    let block_2_blobs = vec![blob_1, blob_2, blob_3];
    let block_3_blobs = vec![blob_4];
    let block_4_blobs = vec![blob_5];

    blob_storage.store_blobs(2, block_2_blobs.clone(), &mut working_set);
    blob_storage.store_blobs(3, block_3_blobs.clone(), &mut working_set);
    blob_storage.store_blobs(4, block_4_blobs.clone(), &mut working_set);

    assert_eq!(
        block_2_blobs,
        blob_storage.take_blobs_for_block_number(2, &mut working_set)
    );
    assert!(blob_storage
        .take_blobs_for_block_number(2, &mut working_set)
        .is_empty());

    assert_eq!(
        block_3_blobs,
        blob_storage.take_blobs_for_block_number(3, &mut working_set)
    );
    assert!(blob_storage
        .take_blobs_for_block_number(3, &mut working_set)
        .is_empty());

    assert_eq!(
        block_4_blobs,
        blob_storage.take_blobs_for_block_number(4, &mut working_set)
    );
    assert!(blob_storage
        .take_blobs_for_block_number(4, &mut working_set)
        .is_empty());
}
