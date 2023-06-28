use super::SovU256;
use bytes::Bytes;
use revm::{
    self,
    primitives::{
        CfgEnv, CreateScheme, EVMError, Env, ExecutionResult, TransactTo, TxEnv, B160, B256, U256,
    },
    Database, DatabaseCommit,
};
use std::convert::Infallible;

#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct BlockEnv {
    pub number: SovU256,
    /// Coinbase or miner or address that created and signed the block.
    /// Address where we are going to send gas spend
    pub coinbase: [u8; 20],
    pub timestamp: SovU256,

    /// Prevrandao is used after Paris (aka TheMerge) instead of the difficulty value.
    /// NOTE: prevrandao can be found in block in place of mix_hash.
    pub prevrandao: Option<SovU256>,
    /// basefee is added in EIP1559 London upgrade
    pub basefee: SovU256,
    pub gas_limit: SovU256,
}

impl Default for BlockEnv {
    fn default() -> Self {
        Self {
            number: Default::default(),
            coinbase: Default::default(),
            timestamp: Default::default(),
            prevrandao: Some([0; 32]),
            basefee: Default::default(),
            gas_limit: [255; 32],
        }
    }
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct AccessListItem {
    pub address: [u8; 20],
    pub storage_keys: Vec<SovU256>,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct EvmTransaction {
    pub caller: [u8; 20],
    pub data: Vec<u8>,
    pub gas_limit: u64,
    pub gas_price: Option<SovU256>,
    pub max_priority_fee_per_gas: Option<SovU256>,
    pub to: Option<[u8; 20]>,
    pub value: SovU256,
    pub nonce: u64,
    pub access_lists: Vec<AccessListItem>,
}

impl Default for EvmTransaction {
    fn default() -> Self {
        Self {
            caller: Default::default(),
            data: Default::default(),
            gas_limit: u64::MAX,
            gas_price: Default::default(),
            max_priority_fee_per_gas: Default::default(),
            to: Default::default(),
            value: Default::default(),
            nonce: Default::default(),
            access_lists: Default::default(),
        }
    }
}

#[allow(dead_code)]
pub(crate) fn execute_tx<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block_env: BlockEnv,
    tx: EvmTransaction,
    config_env: CfgEnv,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let mut evm = revm::new();

    let env = Env {
        cfg: config_env,
        block: block_env.into(),
        tx: tx.into(),
    };

    evm.env = env;
    evm.database(db);
    evm.transact_commit()
}
