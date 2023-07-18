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

    assert!(blob_storage
        .earliest_stored_block_number(&mut working_set)
        .is_none());

    let blobs = blob_storage
        .get_blobs_for_block_number(1, &mut working_set)
        .unwrap();

    assert!(blobs.is_empty());
}

#[test]
fn store_and_retrieve_standard() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let blob_storage = BlobStorage::<C>::default();

    blob_storage.genesis(&(), &mut working_set).unwrap();

    assert!(blob_storage
        .earliest_stored_block_number(&mut working_set)
        .is_none());

    let blob_1 = vec![1, 2, 3];
    let blob_2 = vec![3, 4, 5];
    let blob_3 = vec![6, 7, 8];
    let blob_4 = vec![9, 9, 9];
    let blob_5 = vec![0, 1, 0];

    blob_storage
        .store_blob(2, blob_1.clone(), &mut working_set)
        .unwrap();
    blob_storage
        .store_blob(2, blob_2.clone(), &mut working_set)
        .unwrap();
    blob_storage
        .store_blob(2, blob_3.clone(), &mut working_set)
        .unwrap();
    assert_eq!(
        Some(2),
        blob_storage.earliest_stored_block_number(&mut working_set)
    );
    blob_storage
        .store_blob(3, blob_4, &mut working_set)
        .unwrap();
    blob_storage
        .store_blob(4, blob_5, &mut working_set)
        .unwrap();
    assert_eq!(
        Some(2),
        blob_storage.earliest_stored_block_number(&mut working_set)
    );

    let blobs = blob_storage
        .get_blobs_for_block_number(2, &mut working_set)
        .unwrap();

    assert_eq!(vec![blob_1, blob_2, blob_3], blobs);
    assert_eq!(
        Some(3),
        blob_storage.earliest_stored_block_number(&mut working_set)
    );
}

#[test]
fn store_and_retrieve_out_of_order() {
    let tmpdir = tempfile::tempdir().unwrap();
    let mut working_set = WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());
    let blob_storage = BlobStorage::<C>::default();

    blob_storage.genesis(&(), &mut working_set).unwrap();

    let blob_1 = vec![1, 2, 3];
    let blob_2 = vec![3, 4, 5];
    let blob_3 = vec![6, 7, 8];
    let blob_4 = vec![9, 9, 9];
    let blob_5 = vec![0, 1, 0];

    blob_storage
        .store_blob(2, blob_1.clone(), &mut working_set)
        .unwrap();
    blob_storage
        .store_blob(1, blob_2.clone(), &mut working_set)
        .unwrap();
    assert_eq!(
        Some(2),
        blob_storage.earliest_stored_block_number(&mut working_set)
    );
    blob_storage
        .store_blob(2, blob_3.clone(), &mut working_set)
        .unwrap();
    blob_storage
        .store_blob(1, blob_4.clone(), &mut working_set)
        .unwrap();
    blob_storage
        .store_blob(2, blob_5.clone(), &mut working_set)
        .unwrap();
    assert_eq!(
        Some(2),
        blob_storage.earliest_stored_block_number(&mut working_set)
    );

    let blobs_1 = blob_storage
        .get_blobs_for_block_number(1, &mut working_set)
        .unwrap();
    assert_eq!(
        Some(2),
        blob_storage.earliest_stored_block_number(&mut working_set)
    );
    assert_eq!(vec![blob_2, blob_4], blobs_1);
    let blobs_2 = blob_storage
        .get_blobs_for_block_number(2, &mut working_set)
        .unwrap();
    assert_eq!(vec![blob_1, blob_3, blob_5], blobs_2);
    assert!(blob_storage
        .earliest_stored_block_number(&mut working_set)
        .is_none());
}
