use bytes::Bytes;
use ethereum_types::U256 as EU256;
use ethers_contract::BaseContract;
use ethers_core::abi::Abi;
use revm::{
    primitives::{
        AccountInfo, ExecutionResult, Output, TransactTo, TxEnv, B160, KECCAK_EMPTY, U256,
    },
    DummyStateDB,
};

use std::{path::PathBuf, str::FromStr};

use super::{db::EvmDb, executor};

fn output(result: ExecutionResult) -> Bytes {
    match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(out) => out,
            Output::Create(out, _) => out,
        },
        _ => panic!(),
    }
}

#[test]
fn simple_contract_execution() {
    let caller = B160::from_str("0x1000000000000000000000000000000000000000").unwrap();
    let mut db = DummyStateDB::default();

    db.insert_account_info(
        caller,
        AccountInfo {
            nonce: 1,
            balance: U256::from(1000000000),
            code: None,
            code_hash: KECCAK_EMPTY,
        },
    );

    let contract_address = {
        let mut path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("src");
        path.push("evm");
        path.push("sol");
        path.push("build");
        path.push("SimpleStorage.bin");

        let data = std::fs::read_to_string(path).unwrap();
        let data = Bytes::from(hex::decode(data).unwrap());

        let mut tx_env = TxEnv::default();
        tx_env.transact_to = TransactTo::create();
        tx_env.data = data;

        let result = executor::execute_tx(EvmDb { db: &mut db }, tx_env).unwrap();

        match result {
            ExecutionResult::Success {
                output: Output::Create(_, Some(addr)),
                ..
            } => addr,
            _ => panic!(""),
        }
    };

    let set_arg = EU256::from(21989);

    let mut path: PathBuf = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("src");
    path.push("evm");
    path.push("sol");
    path.push("build");
    path.push("SimpleStorage.abi");

    let abi_json = std::fs::read_to_string(path).unwrap();
    let abi: Abi = serde_json::from_str(&abi_json).unwrap();
    let abi = BaseContract::from(abi);

    {
        let encoded = abi.encode("set", set_arg).unwrap();

        let mut tx_env = TxEnv::default();
        tx_env.transact_to = TransactTo::Call(contract_address);
        tx_env.data = Bytes::from(hex::decode(hex::encode(&encoded)).unwrap());

        executor::execute_tx(EvmDb { db: &mut db }, tx_env).unwrap();
    }

    let get_res = {
        let encoded = abi.encode("get", ()).unwrap();
        let mut tx_env = TxEnv::default();
        tx_env.transact_to = TransactTo::Call(contract_address);
        tx_env.data = Bytes::from(hex::decode(hex::encode(&encoded)).unwrap());

        let result = executor::execute_tx(EvmDb { db: &mut db }, tx_env).unwrap();

        let out = output(result);
        EU256::from(out.as_ref())
    };

    assert_eq!(set_arg, get_res)
}
