use crate::{
    call::CallMessage,
    evm::{
        db_init::InitEvmDb, test_helpers::SimpleStorageContract, transaction::EvmTransaction,
        AccountInfo, EthAddress,
    },
    Evm,
};
use revm::primitives::{KECCAK_EMPTY, U256};
use sov_modules_api::{
    default_context::DefaultContext, default_signature::private_key::DefaultPrivateKey, Context,
    Module, PublicKey, Spec,
};
use sov_state::{ProverStorage, WorkingSet};

type C = DefaultContext;

fn create_messages(contract_addr: EthAddress, set_arg: ethereum_types::U256) -> Vec<CallMessage> {
    let mut transactions = Vec::default();
    let contract = SimpleStorageContract::new();

    // Contract creation.
    {
        transactions.push(CallMessage {
            tx: EvmTransaction {
                to: None,
                data: contract.byte_code().to_vec(),
                ..Default::default()
            },
        });
    }

    // Update contract state.
    {
        transactions.push(CallMessage {
            tx: EvmTransaction {
                to: Some(contract_addr),
                data: hex::decode(hex::encode(&contract.set_call_data(set_arg))).unwrap(),
                nonce: 1,
                ..Default::default()
            },
        });
    }

    transactions
}

#[test]
fn evm_test() {
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

    let priv_key = DefaultPrivateKey::generate();

    let sender = priv_key.pub_key();
    let sender_addr = sender.to_address::<<C as Spec>::Address>();
    let sender_context = C::new(sender_addr);
    let caller = [0; 20];

    let evm = Evm::<C>::default();
    let mut evm_db = evm.get_db(working_set);

    evm_db.insert_account_info(
        caller,
        AccountInfo {
            balance: U256::from(1000000000).to_le_bytes(),
            code_hash: KECCAK_EMPTY.to_fixed_bytes(),
            code: vec![],
            nonce: 0,
        },
    );

    let contract_addr = hex::decode("bd770416a3345f91e4b34576cb804a576fa48eb1")
        .unwrap()
        .try_into()
        .unwrap();

    let set_arg = ethereum_types::U256::from(999);

    for tx in create_messages(contract_addr, set_arg) {
        evm.call(tx, &sender_context, working_set).unwrap();
    }

    let db_account = evm.accounts.get(&contract_addr, working_set).unwrap();
    let storage_key = &[0; 32];
    let storage_value = db_account.storage.get(storage_key, working_set).unwrap();

    assert_eq!(
        set_arg,
        ethereum_types::U256::from_little_endian(&storage_value)
    )
}
