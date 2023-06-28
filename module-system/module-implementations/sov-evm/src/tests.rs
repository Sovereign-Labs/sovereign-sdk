use crate::{
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
    PublicKey, Spec,
};
use sov_state::{ProverStorage, WorkingSet};
use std::path::PathBuf;

type C = DefaultContext;

fn contract_address(result: ExecutionResult) -> B160 {
    match result {
        ExecutionResult::Success {
            output: Output::Create(_, Some(addr)),
            ..
        } => addr,
        _ => panic!("Expected successful contract creation"),
    }
}

fn output(result: ExecutionResult) -> Bytes {
    match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(out) => out,
            Output::Create(out, _) => out,
        },
        _ => panic!("Expected successful ExecutionResult"),
    }
}

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

/*
fn transactions() -> Vec<EvmTransaction> {

}*/

#[test]
fn evm_test() {
    let tmpdir = tempfile::tempdir().unwrap();
    let working_set = &mut WorkingSet::new(ProverStorage::with_path(tmpdir.path()).unwrap());

    let priv_key = DefaultPrivateKey::generate();

    let sender = priv_key.pub_key();
    let sender_addr = sender.to_address::<<C as Spec>::Address>();
    let sender_context = C::new(sender_addr.clone());
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

    let mut path = test_data_path();
    path.push("SimpleStorage.bin");

    let contract_data = std::fs::read_to_string(path).unwrap();
    let contract_data = hex::decode(contract_data).unwrap();

    let tx = EvmTransaction {
        to: None,
        data: contract_data,
        ..Default::default()
    };

    let result = evm.execute_tx(tx, &sender_context, working_set).unwrap();
    let contract_address = contract_address(result);

    let set_arg = EU256::from(21989);

    let mut path = test_data_path();
    path.push("SimpleStorage.abi");

    let contract = make_contract_from_abi(path);

    {
        let call_data = contract.encode("set", set_arg).unwrap();

        let tx = EvmTransaction {
            to: Some(*contract_address.as_fixed_bytes()),
            data: hex::decode(hex::encode(&call_data)).unwrap(),
            nonce: 1,
            ..Default::default()
        };

        evm.execute_tx(tx, &sender_context, working_set).unwrap();
    }

    let get_res = {
        let call_data = contract.encode("get", ()).unwrap();

        let tx = EvmTransaction {
            to: Some(*contract_address.as_fixed_bytes()),
            data: hex::decode(hex::encode(&call_data)).unwrap(),
            nonce: 2,
            ..Default::default()
        };

        let result = evm.execute_tx(tx, &sender_context, working_set).unwrap();

        let out = output(result);
        EU256::from(out.as_ref())
    };

    assert_eq!(set_arg, get_res)
}
