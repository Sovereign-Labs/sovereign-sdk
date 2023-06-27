use crate::{
    evm::{db::EvmDb, executor},
    Evm,
};
use anyhow::{bail, Result};
use revm::primitives::{BlockEnv, CfgEnv, Env, TxEnv};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

pub type AccessList = Vec<AccessListItem>;
pub struct AccessListItem {
    //  pub address: B160,
    //  pub storage_keys: Vec<B256>,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct EvmTransaction {
    pub data: Vec<u8>,
    // pub access_lists: Option<Vec<Option<AccessList>>>,
    // pub gas_limit: Vec<U256>,
    // pub gas_price: Option<U256>,
    // pub nonce: U256,
    // pub secret_key: Option<B256>,
    // #[serde(deserialize_with = "deserialize_maybe_empty")]
    // pub to: Option<B160>,
    // pub value: Vec<U256>,
    //pub max_fee_per_gas: Option<U256>,
    // pub max_priority_fee_per_gas: Option<U256>,
}

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct CallMessage {
    txs: EvmTransaction,
}

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn execute_txs(
        &self,
        context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let tx_env = TxEnv {
            caller: todo!(),
            gas_limit: todo!(),
            gas_price: todo!(),
            gas_priority_fee: todo!(),
            transact_to: todo!(),
            value: todo!(),
            data: todo!(),
            chain_id: todo!(),
            nonce: todo!(),
            access_list: todo!(),
        };

        let block_env = BlockEnv {
            number: todo!(),
            coinbase: todo!(),
            timestamp: todo!(),
            difficulty: todo!(),
            prevrandao: todo!(),
            basefee: todo!(),
            gas_limit: todo!(),
        };

        let env = Env {
            cfg: CfgEnv::default(),
            block: block_env,
            tx: tx_env,
        };
        let evm_db: EvmDb<'_, C> = self.get_db(&mut working_set);

        executor::execute_tx(evm_db, env).unwrap();
        Ok(CallResponse::default())
    }
}
