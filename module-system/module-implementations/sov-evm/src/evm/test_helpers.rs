use ethers_contract::BaseContract;
use ethers_core::{abi::Abi, types::Bytes};
use revm::primitives::{ExecutionResult, Output, B160};
use std::path::PathBuf;

pub(crate) fn output(result: ExecutionResult) -> bytes::Bytes {
    match result {
        ExecutionResult::Success { output, .. } => match output {
            Output::Call(out) => out,
            Output::Create(out, _) => out,
        },
        _ => panic!("Expected successful ExecutionResult"),
    }
}

pub(crate) fn contract_address(result: ExecutionResult) -> B160 {
    match result {
        ExecutionResult::Success {
            output: Output::Create(_, Some(addr)),
            ..
        } => addr,
        _ => panic!("Expected successful contract creation"),
    }
}

pub(crate) fn test_data_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("src");
    path.push("evm");
    path.push("test_data");
    path
}

pub(crate) fn make_contract_from_abi(path: PathBuf) -> BaseContract {
    let abi_json = std::fs::read_to_string(path).unwrap();
    let abi: Abi = serde_json::from_str(&abi_json).unwrap();
    BaseContract::from(abi)
}

pub(crate) struct SimpleStorageContract {
    bytecode: Bytes,
    base_contract: BaseContract,
}

impl SimpleStorageContract {
    pub(crate) fn new() -> Self {
        let contract_data = {
            let mut path = test_data_path();
            path.push("SimpleStorage.bin");

            let contract_data = std::fs::read_to_string(path).unwrap();
            hex::decode(contract_data).unwrap()
        };

        let contract = {
            let mut path = test_data_path();
            path.push("SimpleStorage.abi");

            make_contract_from_abi(path)
        };

        Self {
            bytecode: Bytes::from(contract_data),
            base_contract: contract,
        }
    }

    pub(crate) fn byte_code(&self) -> Bytes {
        self.bytecode.clone()
    }

    pub(crate) fn set_call_data(&self, set_arg: ethereum_types::U256) -> Bytes {
        self.base_contract.encode("set", set_arg).unwrap()
    }

    pub(crate) fn get_call_data(&self) -> Bytes {
        self.base_contract.encode("get", ()).unwrap()
    }
}
