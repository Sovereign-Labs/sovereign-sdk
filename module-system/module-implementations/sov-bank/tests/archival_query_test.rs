mod helpers;

use helpers::*;

use sov_bank::{Bank, BankConfig, CallMessage, Coins, get_genesis_token_address};
use sov_modules_api::{Address, Context, Error, Module, Spec, WorkingSet};
use sov_modules_api::utils::generate_address;
use sov_modules_api::default_context::DefaultContext;
use sov_state::{DefaultStorageSpec, ProverStorage, Storage};
use sov_state::storage::NativeStorage;


#[test]
fn transfer_initial_token() {
    let initial_balance = 100;
    let bank_config = create_bank_config_with_token(3, initial_balance);
    let tmpdir = tempfile::tempdir().unwrap();
    let prover_storage = ProverStorage::with_path(tmpdir.path()).unwrap();
    let mut working_set = WorkingSet::new(prover_storage.clone());
    let bank = Bank::default();
    bank.genesis(&bank_config, &mut working_set).unwrap();

    let token_address = get_genesis_token_address::<C>(
        &bank_config.tokens[0].token_name,
        bank_config.tokens[0].salt,
    );
    let sender_address = bank_config.tokens[0].address_and_balances[0].0;
    let receiver_address = bank_config.tokens[0].address_and_balances[1].0;
    assert_ne!(sender_address, receiver_address);

    let (sender_balance, receiver_balance) = query_sender_receiver_balances(&bank, token_address, sender_address, receiver_address, &mut working_set);
    assert_eq!((sender_balance, receiver_balance), (100,100));
    println!("Starting commit for genesis, i.e. slot 1");
    commit(working_set, prover_storage.clone());
    println!("Genesis commit complete");

    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());

    transfer(&bank, token_address, sender_address, receiver_address, &mut working_set);
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(&bank, token_address, sender_address, receiver_address, &mut working_set);
    assert_eq!((sender_balance, receiver_balance), (90,110));

    println!("Starting commit for slot 2");
    commit(working_set, prover_storage.clone());
    println!("Commit complete for slot 2");

    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(prover_storage.clone());

    transfer(&bank, token_address, sender_address, receiver_address, &mut working_set);
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(&bank, token_address, sender_address, receiver_address, &mut working_set);
    assert_eq!((sender_balance, receiver_balance), (80,120));
    println!("Starting commit for slot 3");
    commit(working_set, prover_storage.clone());
    println!("Commit complete for slot 3");


    let archival_slot: u64 = 2;
    println!("Archival reads at slot {}",archival_slot);
    let mut versioned_prover_storage = prover_storage.clone();
    versioned_prover_storage.set_archival_version(archival_slot).expect("TODO: panic message");
    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(versioned_prover_storage.clone());
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(&bank, token_address, sender_address, receiver_address, &mut working_set);
    assert_eq!((sender_balance, receiver_balance), (90,110));

    let archival_slot: u64 = 1;
    println!("Archival archival reads at slot {}",archival_slot);
    let mut versioned_prover_storage = prover_storage.clone();
    versioned_prover_storage.set_archival_version(archival_slot).expect("TODO: panic message");
    let mut working_set: WorkingSet<DefaultContext> = WorkingSet::new(versioned_prover_storage.clone());
    let (sender_balance, receiver_balance) = query_sender_receiver_balances(&bank, token_address, sender_address, receiver_address, &mut working_set);
    assert_eq!((sender_balance, receiver_balance), (100,100));

}

fn query_sender_receiver_balances(bank: &Bank<DefaultContext>,
                                  token_address: Address,
                                  sender_address: Address,
                                  receiver_address: Address,
                                  working_set: &mut WorkingSet<DefaultContext>) -> (u64, u64){
    let query_user_balance =
        |user_address: Address, working_set: &mut WorkingSet<DefaultContext>| -> Option<u64> {
            bank.get_balance_of(user_address, token_address, working_set)
        };

    let sender_balance = query_user_balance(sender_address, working_set).unwrap();
    let receiver_balance = query_user_balance(receiver_address, working_set).unwrap();
    println!("S: {}", &sender_balance);
    println!("R: {}", &receiver_balance);
    return (sender_balance, receiver_balance);
}

fn transfer(bank: &Bank<DefaultContext>,
            token_address: Address,
            sender_address: Address,
            receiver_address: Address,
            working_set: &mut WorkingSet<DefaultContext>) {
    let transfer_amount = 10;
    let transfer_message = CallMessage::Transfer {
        to: receiver_address,
        coins: Coins {
            amount: transfer_amount,
            token_address,
        },
    };

    let sender_context = C::new(sender_address);

    bank.call(transfer_message, &sender_context, working_set)
        .expect("Transfer call failed");
    println!("Transfer complete");
}

fn commit(working_set: WorkingSet<DefaultContext>, storage: ProverStorage<DefaultStorageSpec>) {
    // Save checkpoint
    let mut checkpoint = working_set.checkpoint();

    let (cache_log, witness) = checkpoint.freeze();

    let (_, authenticated_node_batch) = storage
        .compute_state_update(cache_log, &witness)
        .expect("jellyfish merkle tree update must succeed");

    let mut working_set = checkpoint.to_revertable();

    let accessory_log = working_set.checkpoint().freeze_non_provable();

    storage
        .commit(&authenticated_node_batch, &accessory_log);
}