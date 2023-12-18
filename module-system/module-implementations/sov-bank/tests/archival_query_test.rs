mod helpers;

use helpers::*;
use sov_bank::{get_genesis_token_address, Amount, Bank, CallMessage, Coins};
use sov_modules_api::default_context::DefaultContext;
use sov_modules_api::{Address, Context, Module, StateReaderAndWriter, WorkingSet};
use sov_prover_storage_manager::{new_orphan_storage, SnapshotManager};
use sov_state::storage::{StorageKey, StorageValue};
use sov_state::{DefaultStorageSpec, ProverStorage, Storage};

#[test]
fn transfer_initial_token() {
    let initial_balance = 100;
    let bank_config = create_bank_config_with_token(4, initial_balance);
    let tmpdir = tempfile::tempdir().unwrap();
    let prover_storage = new_orphan_storage(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(prover_storage.clone());
    let bank = Bank::default();
    bank.genesis(&bank_config, &mut working_set).unwrap();

    let token_address = get_genesis_token_address::<C>(
        &bank_config.tokens[0].token_name,
        bank_config.tokens[0].salt,
    );
    let sender_address = bank_config.tokens[0].address_and_balances[0].0;
    let sequencer_address = bank_config.tokens[0].address_and_balances[3].0;
    let receiver_address = bank_config.tokens[0].address_and_balances[1].0;
    assert_ne!(sender_address, receiver_address);

    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (100, 100));
    commit(working_set, prover_storage.clone());

    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());

    transfer(
        &bank,
        token_address,
        sender_address,
        sequencer_address,
        receiver_address,
        10,
        &mut working_set,
    );
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (90, 110));

    commit(working_set, prover_storage.clone());

    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());

    transfer(
        &bank,
        token_address,
        sender_address,
        sequencer_address,
        receiver_address,
        10,
        &mut working_set,
    );
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (80, 120));
    commit(working_set, prover_storage.clone());

    // Archival tests

    let archival_slot: u64 = 2;
    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());
    working_set.set_archival_version(archival_slot);

    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (90, 110));

    // modify in archival
    transfer(
        &bank,
        token_address,
        sender_address,
        sequencer_address,
        receiver_address,
        5,
        &mut working_set,
    );

    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (85, 115));

    let archival_slot: u64 = 1;
    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());
    working_set.set_archival_version(archival_slot);
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (100, 100));

    transfer(
        &bank,
        token_address,
        sender_address,
        sequencer_address,
        receiver_address,
        45,
        &mut working_set,
    );

    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (55, 145));

    working_set.unset_archival_version();
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (80, 120));

    // Accessory tests

    transfer(
        &bank,
        token_address,
        sender_address,
        sequencer_address,
        receiver_address,
        10,
        &mut working_set,
    );
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (70, 130));

    let mut accessory_state = working_set.accessory_state();
    accessory_state.set(&StorageKey::from("k"), StorageValue::from(b"v1".to_vec()));
    let val = accessory_state.get(&StorageKey::from("k")).unwrap();
    assert_eq!("v1", String::from_utf8(val.value().to_vec()).unwrap());

    commit(working_set, prover_storage.clone());

    // next block

    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());
    transfer(
        &bank,
        token_address,
        sender_address,
        sequencer_address,
        receiver_address,
        10,
        &mut working_set,
    );
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(
        &bank,
        token_address,
        sender_address,
        receiver_address,
        &mut working_set,
    );
    assert_eq!((sender_balance, receiver_balance), (60, 140));
    let mut accessory_state = working_set.accessory_state();
    accessory_state.set(&StorageKey::from("k"), StorageValue::from(b"v2".to_vec()));
    let val = accessory_state.get(&StorageKey::from("k")).unwrap();
    assert_eq!("v2", String::from_utf8(val.value().to_vec()).unwrap());

    commit(working_set, prover_storage.clone());

    // archival versioned state query

    let archival_slot = 3;
    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());
    working_set.set_archival_version(archival_slot);
    let mut accessory_state = working_set.accessory_state();
    let val = accessory_state.get(&StorageKey::from("k")).unwrap();
    assert_eq!("v1", String::from_utf8(val.value().to_vec()).unwrap());

    // archival accessory set

    accessory_state.set(&StorageKey::from("k"), StorageValue::from(b"v3".to_vec()));
    let val = accessory_state.get(&StorageKey::from("k")).unwrap();
    assert_eq!("v3", String::from_utf8(val.value().to_vec()).unwrap());

    working_set.unset_archival_version();
    let mut accessory_state = working_set.accessory_state();
    let val = accessory_state.get(&StorageKey::from("k")).unwrap();
    assert_eq!("v2", String::from_utf8(val.value().to_vec()).unwrap());
}

fn query_sender_receiver_balances(
    bank: &Bank<DefaultContext>,
    token_address: Address,
    sender_address: Address,
    receiver_address: Address,
    working_set: &mut WorkingSet<DefaultContext>,
) -> (u64, u64) {
    let sender_balance = bank
        .get_balance_of(sender_address, token_address, working_set)
        .unwrap();
    let receiver_balance = bank
        .get_balance_of(receiver_address, token_address, working_set)
        .unwrap();
    (sender_balance, receiver_balance)
}

fn transfer(
    bank: &Bank<DefaultContext>,
    token_address: Address,
    sender_address: Address,
    sequencer_address: Address,
    receiver_address: Address,
    transfer_amount: Amount,
    working_set: &mut WorkingSet<DefaultContext>,
) {
    let transfer_message = CallMessage::Transfer {
        to: receiver_address,
        coins: Coins {
            amount: transfer_amount,
            token_address,
        },
    };

    let sender_context = C::new(sender_address, sequencer_address, 1);

    bank.call(transfer_message, &sender_context, working_set)
        .expect("Transfer call failed");
}

fn commit(
    working_set: WorkingSet<DefaultContext>,
    storage: ProverStorage<DefaultStorageSpec, SnapshotManager>,
) {
    // Save checkpoint
    let mut checkpoint = working_set.checkpoint();

    let (cache_log, witness) = checkpoint.freeze();

    let (_, authenticated_node_batch) = storage
        .compute_state_update(cache_log, &witness)
        .expect("jellyfish merkle tree update must succeed");

    let working_set = checkpoint.to_revertable();

    let accessory_log = working_set.checkpoint().freeze_non_provable();

    storage.commit(&authenticated_node_batch, &accessory_log);
}
