use std::path::PathBuf;

use ethers::contract::BaseContract;
use ethers::core::abi::Abi;
use ethers::core::types::Bytes;

fn test_data_path() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("src");
    path.push("evm");
    path.push("test-data");
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

    /// Inc function for the smart contract.
    pub fn inc_call_data(&self) -> Bytes {
        self.base_contract.encode("inc", ()).unwrap()
    }

    /// Failing call data to test revert.
    pub fn failing_function_call_data(&self) -> Bytes {
        // Some random function signature.
        let data = hex::decode("a5643bf2").unwrap();
        Bytes::from(data)
    }

    /// Revert transaction.
    pub fn always_revert(&self) -> Bytes {
        self.base_contract.encode("alwaysRevert", ()).unwrap()
    }

    /// Emit logss.
    pub fn emit_logs(&self, topic: u32, nb_of_logs: u32) -> Bytes {
        let topic = ethereum_types::U256::from(topic);
        let nb_of_logs = ethereum_types::U256::from(nb_of_logs);
        self.base_contract
            .encode("emitLogs", (topic, nb_of_logs))
            .unwrap()
    }
}

use alloy_sol_types::sol;
use alloy_sol_types::SolEvent;

/// Log with some additional metadata.
#[derive(Debug, Clone)]
pub struct SimpleStorageContractLog {
    pub paresed: SimpleLog,
    pub original: alloy_rpc_types_eth::Log,
}

sol! {
    #[derive(Debug)]
    event SimpleLog(address indexed sender,uint256 indexed topic,uint256 value);
}

impl SimpleStorageContract {
    /// Decode log
    pub fn decode_alloy(log: alloy_rpc_types_eth::Log) -> SimpleStorageContractLog {
        let decoded_log = SimpleLog::decode_log_validate(&log.inner).unwrap();
        SimpleStorageContractLog {
            paresed: decoded_log.data,
            original: log,
        }
    }
}
