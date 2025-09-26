use ethereum_types::Address;
use ethereum_types::U256;
use ethers::contract::BaseContract;
use ethers::core::types::Bytes;

use crate::evm::make_contract_from_abi;
use crate::evm::test_data_path;

/// ERC20 wrapper
pub struct ERC20 {
    bytecode: Bytes,
    base_contract: BaseContract,
}

impl Default for ERC20 {
    fn default() -> Self {
        let contract_data = {
            let mut path = test_data_path();
            path.push("ERC20.bin");
            let contract_data = std::fs::read_to_string(path).unwrap();
            hex::decode(contract_data).unwrap()
        };

        let contract = {
            let mut path = test_data_path();
            path.push("ERC20.abi");
            make_contract_from_abi(path)
        };

        Self {
            bytecode: Bytes::from(contract_data),
            base_contract: contract,
        }
    }
}

impl ERC20 {
    /// Returns bytecode
    pub fn byte_code(&self) -> Bytes {
        self.bytecode.clone()
    }

    /// Mints tokens
    pub fn mint(&self, address: Address, amount: U256) -> Bytes {
        self.base_contract
            .encode("mint", (address, amount))
            .unwrap()
    }
}
