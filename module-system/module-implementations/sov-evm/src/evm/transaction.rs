use super::{Bytes32, EthAddress};

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub(crate) struct BlockEnv {
    pub(crate) number: Bytes32,
    pub(crate) coinbase: EthAddress,
    pub(crate) timestamp: Bytes32,
    /// Prevrandao is used after Paris (aka TheMerge) instead of the difficulty value.
    pub(crate) prevrandao: Option<Bytes32>,
    /// basefee is added in EIP1559 London upgrade
    pub(crate) basefee: Bytes32,
    pub(crate) gas_limit: Bytes32,
}

impl Default for BlockEnv {
    fn default() -> Self {
        Self {
            number: Default::default(),
            coinbase: Default::default(),
            timestamp: Default::default(),
            prevrandao: Some(Default::default()),
            basefee: Default::default(),
            gas_limit: [u8::MAX; 32],
        }
    }
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct AccessListItem {
    pub address: EthAddress,
    pub storage_keys: Vec<Bytes32>,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct EvmTransaction {
    pub sender: EthAddress,
    pub data: Vec<u8>,
    pub gas_limit: u64,
    pub gas_price: Bytes32,
    pub max_priority_fee_per_gas: Bytes32,
    pub max_fee_per_gas: Bytes32,
    pub to: Option<[u8; 20]>,
    pub value: Bytes32,
    pub nonce: u64,
    pub access_lists: Vec<AccessListItem>,
    pub chain_id: u64,
    pub odd_y_parity: bool,
    pub r: [u8; 32],
    pub s: [u8; 32],
    // todo remove it
    pub hash: [u8; 32],
}

impl Default for EvmTransaction {
    fn default() -> Self {
        Self {
            sender: Default::default(),
            data: Default::default(),
            gas_limit: u64::MAX,
            gas_price: Default::default(),
            max_priority_fee_per_gas: Default::default(),
            max_fee_per_gas: Default::default(),
            to: Default::default(),
            value: Default::default(),
            nonce: Default::default(),
            access_lists: Default::default(),
            chain_id: 1,
            hash: Default::default(),
            odd_y_parity: Default::default(),
            r: Default::default(),
            s: Default::default(),
        }
    }
}
