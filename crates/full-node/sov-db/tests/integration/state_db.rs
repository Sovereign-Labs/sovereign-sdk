use std::sync::Arc;

use jmt::storage::TreeReader;
use jmt::{JellyfishMerkleTree, KeyHash};
use rockbound::cache::delta_reader::DeltaReader;
use sov_db::namespaces::{KernelNamespace, Namespace, UserNamespace};
use sov_db::state_db::{JmtHandler, StateDb};
use sov_db::test_utils::build_data_to_materialize;
use sov_rollup_interface::common::IntoSlotNumber;

type H = sha2::Sha256;

#[test]
/// Test case checks basic usage of StateDb public API: materializing data and reading it back.
/// It also checks that the correct value is written to its own namespace.
fn test_state_db_simple() {
    let tempdir = tempfile::tempdir().unwrap();
    let rocksdb = Arc::new(
        StateDb::get_rockbound_options()
            .default_setup_db_in_path(tempdir.path())
            .unwrap(),
    );
    let state_db = init_state_db(rocksdb.clone());

    let key_hash = KeyHash([1u8; 32]);
    let key = vec![2u8; 128];
    let value_1 = [8u8; 192];
    let value_2 = [9u8; 192];

    // Writing
    let preimages_schematized =
        StateDb::materialize_preimages(vec![(key_hash, &key)], vec![(key_hash, &key)]).unwrap();

    let user_materialize = build_data_to_materialize::<_, H>(
        &state_db.get_jmt_handler::<UserNamespace>(),
        0,
        vec![(key_hash, Some(value_1.to_vec()))],
    );
    let kernel_materialize = build_data_to_materialize::<_, H>(
        &state_db.get_jmt_handler::<KernelNamespace>(),
        0,
        vec![(key_hash, Some(value_2.to_vec()))],
    );
    let batch = state_db
        .materialize(
            &kernel_materialize,
            &user_materialize,
            Some(preimages_schematized),
        )
        .unwrap();
    rocksdb.write_schemas(&batch).unwrap();

    // Still zero after materialization and writing.
    // Nothing changes
    assert_eq!(state_db.get_next_version(), 0.to_slot_number());
    for version in [0, 1, 2, u64::MAX] {
        assert!(state_db
            .get_jmt_handler::<UserNamespace>()
            .get_value(version, key_hash)
            .is_err());
        assert!(state_db
            .get_jmt_handler::<KernelNamespace>()
            .get_value(version, key_hash)
            .is_err());
    }
    // Reading back gives nothing, as StateDB is effectively still empty
    assert!(state_db
        .get_jmt_handler::<UserNamespace>()
        .get_value(0, key_hash)
        .is_err());
    assert!(state_db
        .get_jmt_handler::<KernelNamespace>()
        .get_value(0, key_hash)
        .is_err());

    let state_db = init_state_db(rocksdb.clone());
    assert_eq!(state_db.get_next_version(), 1.to_slot_number());
    assert_eq!(
        state_db
            .get_jmt_handler::<UserNamespace>()
            .get_value(0, key_hash)
            .unwrap(),
        value_1
    );
    assert_eq!(
        state_db
            .get_jmt_handler::<KernelNamespace>()
            .get_value(0, key_hash)
            .unwrap(),
        value_2
    );

    for version in [1, 2, u64::MAX] {
        assert!(state_db
            .get_jmt_handler::<UserNamespace>()
            .get_value(version, key_hash)
            .is_err());
        assert!(state_db
            .get_jmt_handler::<KernelNamespace>()
            .get_value(version, key_hash)
            .is_err());
    }
}

#[test]
/// Test case checks that an empty rollup batch can be written
/// and the version should increase the next_version.
fn test_state_db_writing_empty_batch() {
    let tempdir = tempfile::tempdir().unwrap();
    let rocksdb = Arc::new(
        StateDb::get_rockbound_options()
            .default_setup_db_in_path(tempdir.path())
            .unwrap(),
    );
    let state_db = init_state_db(rocksdb.clone());

    let user_materialize = build_data_to_materialize::<_, H>(
        &state_db.get_jmt_handler::<UserNamespace>(),
        0,
        Vec::new(),
    );
    let kernel_materialize = build_data_to_materialize::<_, H>(
        &state_db.get_jmt_handler::<KernelNamespace>(),
        0,
        Vec::new(),
    );

    let batch = state_db
        .materialize(&kernel_materialize, &user_materialize, None)
        .unwrap();
    rocksdb.write_schemas(&batch).unwrap();

    // Still zero after materialization and writing.
    // Nothing changes.
    assert_eq!(state_db.get_next_version(), 0.to_slot_number());

    let state_db = init_state_db(rocksdb.clone());
    assert_eq!(state_db.get_next_version(), 1.to_slot_number());
}

#[test]
/// Test case checks that an empty [`NodeBatch`] should not be allowed.
/// Note that [`NodeBatch`] is not equal to rollup batch.
fn test_state_db_empty_nodes() {
    let tempdir = tempfile::tempdir().unwrap();
    let rocksdb = Arc::new(
        StateDb::get_rockbound_options()
            .default_setup_db_in_path(tempdir.path())
            .unwrap(),
    );
    let state_db = init_state_db(rocksdb.clone());

    let materializing_error = state_db
        .materialize(
            &Default::default(),
            &build_data_to_materialize::<_, H>(
                &state_db.get_jmt_handler::<UserNamespace>(),
                0,
                Vec::new(),
            ),
            None,
        )
        .unwrap_err();

    assert_eq!(
        "NodeBatch sov_db::namespaces::KernelNamespace should have at least one Node",
        materializing_error.to_string()
    );

    let materializing_error = state_db
        .materialize(
            &build_data_to_materialize::<_, H>(
                &state_db.get_jmt_handler::<KernelNamespace>(),
                0,
                Vec::new(),
            ),
            &Default::default(),
            None,
        )
        .unwrap_err();

    assert_eq!(
        "NodeBatch sov_db::namespaces::UserNamespace should have at least one Node",
        materializing_error.to_string()
    );

    // Nothing changes.
    assert_eq!(state_db.get_next_version(), 0.to_slot_number());
}

#[test]
/// Test case shows what happen if same [`NodeBatch`] is passed to both namespaces.
fn test_state_db_writing_same_node_batch_both_namespaces() {
    let tempdir = tempfile::tempdir().unwrap();
    let rocksdb = Arc::new(
        StateDb::get_rockbound_options()
            .default_setup_db_in_path(tempdir.path())
            .unwrap(),
    );
    let state_db = init_state_db(rocksdb.clone());
    let key_hash = KeyHash([1u8; 32]);
    let key = vec![2u8; 128];
    let value_1 = [8u8; 192];

    let preimages_schematized =
        StateDb::materialize_preimages(vec![(key_hash, &key)], vec![(key_hash, &key)]).unwrap();

    let user_materialize = build_data_to_materialize::<_, H>(
        &state_db.get_jmt_handler::<UserNamespace>(),
        0,
        vec![(key_hash, Some(value_1.to_vec()))],
    );

    // Oops!
    let batch = state_db
        .materialize(
            &user_materialize,
            &user_materialize,
            Some(preimages_schematized),
        )
        .unwrap();
    rocksdb.write_schemas(&batch).unwrap();

    // So that's seems fine, user value leaks into kernel data.
    let state_db = init_state_db(rocksdb.clone());
    assert_eq!(state_db.get_next_version(), 1.to_slot_number());

    assert_eq!(
        state_db
            .get_jmt_handler::<UserNamespace>()
            .get_value(0, key_hash)
            .unwrap(),
        value_1
    );
    assert_eq!(
        state_db
            .get_jmt_handler::<KernelNamespace>()
            .get_value(0, key_hash)
            .unwrap(),
        value_1
    );
}

#[test]
/// User namespace is materialized at version 0, but kernel is at version 1.
/// Let's see what happens.
fn test_write_node_batches_different_versions() {
    let tempdir = tempfile::tempdir().unwrap();
    let rocksdb = Arc::new(
        StateDb::get_rockbound_options()
            .default_setup_db_in_path(tempdir.path())
            .unwrap(),
    );
    let state_db = init_state_db(rocksdb.clone());

    let key_hash = KeyHash([1u8; 32]);
    let key = vec![2u8; 128];
    let value_1 = [8u8; 192];
    let value_2 = [9u8; 192];

    // Writing
    let preimages_schematized =
        StateDb::materialize_preimages(vec![(key_hash, &key)], vec![(key_hash, &key)]).unwrap();

    let user_node_batch = build_data_to_materialize::<_, H>(
        &state_db.get_jmt_handler::<UserNamespace>(),
        0,
        vec![(key_hash, Some(value_1.to_vec()))],
    );
    let kernel_node_batch = build_data_to_materialize::<_, H>(
        &state_db.get_jmt_handler::<KernelNamespace>(),
        1,
        vec![(key_hash, Some(value_2.to_vec()))],
    );
    let batch = state_db
        .materialize(
            &kernel_node_batch,
            &user_node_batch,
            Some(preimages_schematized),
        )
        .unwrap();

    rocksdb.write_schemas(&batch).unwrap();

    // What is the next version going to be?
    let reader = DeltaReader::new(rocksdb, Vec::new());
    // Oops. It will be decided later if this should be fixed or not.
    let err = StateDb::with_delta_reader(reader).unwrap_err();
    assert_eq!(
        "Kernel and User namespaces have different largest versions: kernel=Some(1), user=Some(0)",
        err.to_string()
    );
}

#[test]
/// Test case is similar to `test_state_db_simple`, but name spaces values are written separately
fn test_namespace() {
    let tempdir = tempfile::tempdir().unwrap();
    let rocksdb = Arc::new(
        StateDb::get_rockbound_options()
            .default_setup_db_in_path(tempdir.path())
            .unwrap(),
    );
    let state_db = init_state_db(rocksdb.clone());

    let key_hash = KeyHash([1u8; 32]);
    let key = vec![2u8; 100];
    let value_1 = [8u8; 150];
    let value_2 = [100u8; 150];
    let version_0 = 0;
    let version_1 = 1;

    // Populate only the user space of the state db with some values, but not kernel.
    // StateDb public API allows that.
    {
        // Note, we populate both user/kernel preimages here for more honest testing.
        let preimages_schematized =
            StateDb::materialize_preimages(vec![(key_hash, &key)], vec![(key_hash, &key)]).unwrap();
        let user_batch = build_data_to_materialize::<_, H>(
            &state_db.get_jmt_handler::<UserNamespace>(),
            version_0,
            vec![(key_hash, Some(value_1.to_vec()))],
        );
        let kernel_node_batch = build_data_to_materialize::<_, H>(
            &state_db.get_jmt_handler::<KernelNamespace>(),
            version_0,
            Vec::new(),
        );
        let node_batch_schematized = state_db
            .materialize(&kernel_node_batch, &user_batch, Some(preimages_schematized))
            .unwrap();

        rocksdb.write_schemas(&node_batch_schematized).unwrap();
    }

    let state_db = init_state_db(rocksdb.clone());

    // Check that user space values are read correctly from the database.
    assert_eq!(
        state_db
            .get_jmt_handler::<UserNamespace>()
            .get_value(version_0, key_hash)
            .unwrap(),
        value_1
    );

    // Try to retrieve these values from the kernel space
    assert!(state_db
        .get_jmt_handler::<KernelNamespace>()
        .get_value(version_0, key_hash)
        .is_err());

    let state_db = init_state_db(rocksdb.clone());
    // Populate the kernel space of the state db with some values but for different version
    {
        let user_batch = build_data_to_materialize::<_, H>(
            &state_db.get_jmt_handler::<UserNamespace>(),
            version_1,
            Vec::new(),
        );
        let kernel_node_batch = build_data_to_materialize::<_, H>(
            &state_db.get_jmt_handler::<KernelNamespace>(),
            version_1,
            vec![(key_hash, Some(value_2.to_vec()))],
        );
        let node_batch_schematized = state_db
            .materialize(&kernel_node_batch, &user_batch, None)
            .unwrap();

        rocksdb.write_schemas(&node_batch_schematized).unwrap();
    }

    // Check that the correct value is returned.
    let state_db = init_state_db(rocksdb.clone());
    assert_eq!(
        state_db
            .get_jmt_handler::<UserNamespace>()
            .get_value(version_1, key_hash)
            .unwrap(),
        value_1
    );
    assert_eq!(
        state_db
            .get_jmt_handler::<KernelNamespace>()
            .get_value(version_1, key_hash)
            .unwrap(),
        value_2
    );
    assert!(state_db
        .get_jmt_handler::<KernelNamespace>()
        .get_value(version_0, key_hash)
        .is_err());
}

#[test]
fn test_root_hash_at_init() {
    let tempdir = tempfile::tempdir().unwrap();
    let rocksdb = Arc::new(
        StateDb::get_rockbound_options()
            .default_setup_db_in_path(tempdir.path())
            .unwrap(),
    );
    let state_db = init_state_db(rocksdb.clone());
    assert_eq!(0.to_slot_number(), state_db.get_next_version());

    let user_state_db_handler: JmtHandler<'_, UserNamespace> = state_db.get_jmt_handler();
    let kernel_state_db_handler: JmtHandler<'_, KernelNamespace> = state_db.get_jmt_handler();

    check_root_hash_at_init_handler(&user_state_db_handler);
    check_root_hash_at_init_handler(&kernel_state_db_handler);
}

fn check_root_hash_at_init_handler<N: Namespace>(handler: &JmtHandler<N>) {
    let jmt = JellyfishMerkleTree::<JmtHandler<N>, H>::new(handler);

    // Just pointing out the obvious.
    let root_hash = jmt.get_root_hash_option(0).unwrap();
    assert!(root_hash.is_none());
}

fn init_state_db(rocksdb: Arc<rockbound::DB>) -> StateDb {
    let reader = DeltaReader::new(rocksdb, Vec::new());
    StateDb::with_delta_reader(reader).unwrap()
}
