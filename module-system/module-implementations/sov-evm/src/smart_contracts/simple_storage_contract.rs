use std::path::PathBuf;

use ethers_contract::BaseContract;
use ethers_core::abi::Abi;
use ethers_core::types::Bytes;

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

/// SimpleStorageContract wrapper.
pub struct SimpleStorageContract {
    bytecode: Bytes,
    base_contract: BaseContract,
}

impl Default for SimpleStorageContract {
    fn default() -> Self {
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
}

impl SimpleStorageContract {
    /// SimpleStorage bytecode.
    pub fn byte_code(&self) -> Bytes {
        self.bytecode.clone()
    }

    /// Setter for the smart contract.
    pub fn set_call_data(&self, set_arg: u32) -> Bytes {
        let set_arg = ethereum_types::U256::from(set_arg);
        self.base_contract.encode("set", set_arg).unwrap()
    }

    /// Getter for the smart contract.
    pub fn get_call_data(&self) -> Bytes {
        self.base_contract.encode("get", ()).unwrap()
    }

    /// Failing call data to test revert.
    pub fn failing_function_call_data(&self) -> Bytes {
        // Some random function signature.
        let data = hex::decode("a5643bf2").unwrap();
        Bytes::from(data)
    }
}
