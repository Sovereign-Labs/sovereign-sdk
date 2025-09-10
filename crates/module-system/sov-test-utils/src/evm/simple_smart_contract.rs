use std::path::PathBuf;

use ethers::abi::RawLog;
use ethers::contract::BaseContract;
use ethers::contract::EthEvent;
use ethers::core::abi::Abi;
use ethers::core::types::Address;
use ethers::core::types::Bytes;
use ethers::core::types::Log;
use ethers::core::types::U256;

/// Log emited by SimpleStorageContract/
#[derive(Debug, Clone, EthEvent)]
#[ethevent(name = "SimpleLog", abi = "Transfer(address,uint256)")]
pub struct SimpleLog {
    #[ethevent(indexed)]
    pub address: Address,
    pub value: U256,
}

/// Log with some additional metadata.
pub struct SimpleStorageContractLog {
    pub paresed: SimpleLog,
    pub original: Log,
}

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

    /// Emit example log.
    pub fn emit_one_log(&self) -> Bytes {
        self.base_contract.encode("emitOneLog", ()).unwrap()
    }

    /// Emit example log.
    pub fn emit_two_logs(&self) -> Bytes {
        self.base_contract.encode("emitTwoLogs", ()).unwrap()
    }

    /// Parse smart contract log.
    pub fn parse_simple_log(log: Log) -> SimpleStorageContractLog {
        let raw_log = RawLog {
            topics: log.topics.to_vec(),
            data: log.data.to_vec(),
        };

        SimpleStorageContractLog {
            paresed: SimpleLog::decode_log(&raw_log).unwrap(),
            original: log,
        }
    }
}
