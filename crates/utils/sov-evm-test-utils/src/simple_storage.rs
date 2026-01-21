use alloy_primitives::{Bytes, U256};
use alloy_sol_types::{sol, SolCall, SolEvent};

sol!(
    #[sol(
        rpc,
        all_derives = true,
        bytecode = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/contracts/artifacts/", "SimpleStorage.bin")))]
    SimpleStorage,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/contracts/artifacts/",
        "SimpleStorage.abi"
    )
);

/// SimpleStorageContract wrapper for offline encoding.
pub struct LegacySimpleStorage {
    bytecode: Bytes,
}

impl Default for LegacySimpleStorage {
    fn default() -> Self {
        // Use the bytecode already embedded via sol! macro
        Self {
            bytecode: Bytes::from(SimpleStorage::BYTECODE.to_vec()),
        }
    }
}

impl LegacySimpleStorage {
    /// SimpleStorage bytecode.
    pub fn byte_code(&self) -> Bytes {
        self.bytecode.clone()
    }

    /// Setter for the smart contract.
    pub fn set(&self, set_arg: u32) -> Bytes {
        let call = SimpleStorage::setCall {
            _num: U256::from(set_arg),
        };
        Bytes::from(call.abi_encode())
    }

    /// Getter for the smart contract.
    pub fn get(&self) -> Bytes {
        let call = SimpleStorage::getCall {};
        Bytes::from(call.abi_encode())
    }

    /// Inc function for the smart contract.
    pub fn inc(&self) -> Bytes {
        let call = SimpleStorage::incCall {};
        Bytes::from(call.abi_encode())
    }

    /// Failing call data to test revert.
    pub fn failing_function(&self) -> Bytes {
        // Some random function signature.
        Bytes::from(hex::decode("a5643bf2").unwrap())
    }

    /// Revert transaction.
    pub fn always_revert(&self) -> Bytes {
        let call = SimpleStorage::alwaysRevertCall {};
        Bytes::from(call.abi_encode())
    }

    /// Emit logs.
    pub fn emit_logs(&self, topic: u32, nb_of_logs: u32) -> Bytes {
        let call = SimpleStorage::emitLogsCall {
            topic1: U256::from(topic),
            n: U256::from(nb_of_logs),
        };
        Bytes::from(call.abi_encode())
    }

    /// Emit a log with all 4 topic slots populated (max EVM allows).
    pub fn emit_full_topic_log(&self, t0: U256, t1: U256, t2: U256, data: U256) -> Bytes {
        let call = SimpleStorage::emitFullTopicLogCall { t0, t1, t2, data };
        Bytes::from(call.abi_encode())
    }

    /// Emit logs with configurable topic values for flexible testing.
    pub fn emit_configurable_logs(
        &self,
        topic1_base: U256,
        topic2_base: U256,
        count: u32,
    ) -> Bytes {
        let call = SimpleStorage::emitConfigurableLogsCall {
            topic1Base: topic1_base,
            topic2Base: topic2_base,
            count: U256::from(count),
        };
        Bytes::from(call.abi_encode())
    }

    /// Emit a log with no indexed topics (only event signature in topic0).
    pub fn emit_data_only_log(&self, v1: U256, v2: U256) -> Bytes {
        let call = SimpleStorage::emitDataOnlyLogCall { v1, v2 };
        Bytes::from(call.abi_encode())
    }

    /// Burn gas by computing keccak256 in a loop (for gas usage testing).
    pub fn burn_gas(&self, iterations: u32) -> Bytes {
        let call = SimpleStorage::burnGasCall {
            iterations: U256::from(iterations),
        };
        Bytes::from(call.abi_encode())
    }
}

/// Log with some additional metadata.
#[derive(Debug, Clone)]
pub struct SimpleStorageContractLog {
    pub parsed: SimpleLog,
    pub original: alloy_rpc_types_eth::Log,
}

sol! {
    #[derive(Debug)]
    event SimpleLog(address indexed sender,uint256 indexed topic1,uint256 indexed topic2,uint256 value);
    event FullTopicLog(uint256 indexed topic0,uint256 indexed topic1,uint256 indexed topic2,uint256 data);
    event DataOnlyLog(uint256 value1,uint256 value2);
}

impl LegacySimpleStorage {
    /// Decode log
    pub fn decode_alloy(log: alloy_rpc_types_eth::Log) -> SimpleStorageContractLog {
        let decoded_log = SimpleLog::decode_log_validate(&log.inner).unwrap();
        SimpleStorageContractLog {
            parsed: decoded_log.data,
            original: log,
        }
    }
}
