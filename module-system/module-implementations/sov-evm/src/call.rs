use anyhow::Result;
use revm::primitives::{CfgEnv, U256};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::evm::db::EvmDb;
use crate::evm::executor::{self};
use crate::evm::transaction::{BlockEnv, EvmTransaction};
use crate::evm::{contract_address, EvmChainCfg};
use crate::{Evm, TransactionReceipt};

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct CallMessage {
    pub tx: EvmTransaction,
}

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn execute_call(
        &self,
        tx: EvmTransaction,
        _context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/515

        let block_env = self.block_env.get(working_set).unwrap_or_default();
        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let cfg_env = self.get_cfg_env(&block_env, cfg, None);
        self.transactions.set(&tx.hash, &tx, working_set);

        let evm_db: EvmDb<'_, C> = self.get_db(working_set);

        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/505
        let result = executor::execute_tx(evm_db, block_env, tx.clone(), cfg_env).unwrap();

        let receipt = TransactionReceipt {
            transaction_hash: tx.hash,
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            transaction_index: 0,
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            block_hash: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            block_number: Some(0),
            from: tx.sender,
            to: tx.to,
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            cumulative_gas_used: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            gas_used: Default::default(),
            contract_address: contract_address(result).map(|addr| addr.into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            status: Some(1),
            root: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            transaction_type: Some(1),
            effective_gas_price: Default::default(),
        };

        self.receipts
            .set(&receipt.transaction_hash, &receipt, working_set);

        Ok(CallResponse::default())
    }

    pub(crate) fn get_cfg_env(
        &self,
        block_env: &BlockEnv,
        cfg: EvmChainCfg,
        template_cfg: Option<CfgEnv>,
    ) -> CfgEnv {
        let spec = cfg.spec;
        let spec_id = match spec.binary_search_by(|&(k, _)| k.cmp(&block_env.number)) {
            Ok(index) => spec[index].1,
            Err(index) => {
                if index > 0 {
                    spec[index - 1].1
                } else {
                    // this should never happen as we cover this in genesis
                    panic!("EVM spec must start from block 0")
                }
            }
        };

        CfgEnv {
            chain_id: U256::from(cfg.chain_id),
            limit_contract_code_size: cfg.limit_contract_code_size,
            spec_id: spec_id.into(),
            // disable_gas_refund: !cfg.gas_refunds, // option disabled for now, we could add if needed
            ..template_cfg.unwrap_or_default()
        }
    }
}
