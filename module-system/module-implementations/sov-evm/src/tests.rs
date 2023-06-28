use crate::{
    call::CallMessage,
    evm::{db_init::InitEvmDb, executor::EvmTransaction, AccountInfo},
    Evm,
};
use bytes::Bytes;
use ethereum_types::U256 as EU256;
use ethers_contract::BaseContract;
use ethers_core::abi::Abi;
use revm::primitives::{ExecutionResult, Output, B160, KECCAK_EMPTY, U256};
use sov_modules_api::{
    default_context::DefaultContext, default_signature::private_key::DefaultPrivateKey, Context,
    Module, PublicKey, Spec,
};
use sov_state::{ProverStorage, WorkingSet};
use std::path::PathBuf;

type C = DefaultContext;

fn test_data_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("src");
    path.push("evm");
    path.push("test_data");
    path
}

fn make_contract_from_abi(path: PathBuf) -> BaseContract {
    let abi_json = std::fs::read_to_string(path).unwrap();
    let abi: Abi = serde_json::from_str(&abi_json).unwrap();
    BaseContract::from(abi)
}

fn transactions() -> Vec<CallMessage> {
    let mut path = test_data_path();
    path.push("SimpleStorage.bin");

    let contract_data = std::fs::read_to_string(path).unwrap();
    let contract_data = hex::decode(contract_data).unwrap();

    let tx0 = EvmTransaction {
        to: None,
        data: contract_data,
        ..Default::default()
    };

    let set_arg = EU256::from(999);

    let mut path = test_data_path();
    path.push("SimpleStorage.abi");

    let contract = make_contract_from_abi(path);
    let addr: [u8; 20] = hex::decode("bd770416a3345f91e4b34576cb804a576fa48eb1")
        .unwrap()
        .try_into()
        .unwrap();

    let call_data = contract.encode("set", set_arg).unwrap();
    let tx1 = EvmTransaction {
        to: Some(addr),
        data: hex::decode(hex::encode(&call_data)).unwrap(),
        nonce: 1,
        ..Default::default()
    };

    let call_data = contract.encode("get", ()).unwrap();
    let tx2 = EvmTransaction {
        to: Some(addr),
        data: hex::decode(hex::encode(&call_data)).unwrap(),
        nonce: 2,
        ..Default::default()
    };

    vec![CallMessage { tx: tx0 }, CallMessage { tx: tx1 }]
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

    for tx in transactions() {
        evm.call(tx, &sender_context, working_set).unwrap();
    }

    let addr: [u8; 20] = hex::decode("bd770416a3345f91e4b34576cb804a576fa48eb1")
        .unwrap()
        .try_into()
        .unwrap();

    let account = evm.accounts.get(&addr, working_set).unwrap();
    let s = &[0; 32];
    let r = account.storage.get(s, working_set).unwrap();

    let set_arg = EU256::from(999);
    assert_eq!(set_arg, EU256::from_little_endian(&r))
}
