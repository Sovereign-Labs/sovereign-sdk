use anyhow::Result;
use revm::primitives::CfgEnv;
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::evm::db::EvmDb;
use crate::evm::executor::{self};
use crate::evm::transaction::{BlockEnv, EvmTransactionSignedEcRecovered};
use crate::evm::{contract_address, EvmChainCfg, RawEvmTransaction};
use crate::experimental::SpecIdWrapper;
use crate::Evm;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct CallMessage {
    pub tx: RawEvmTransaction,
}

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn execute_call(
        &self,
        tx: RawEvmTransaction,
        _context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let evm_tx_recovered: EvmTransactionSignedEcRecovered = tx.try_into()?;

        let block_env = self.block_env.get(working_set).unwrap_or_default();
        let cfg = self.cfg.get(working_set).unwrap_or_default();
        let cfg_env = get_cfg_env(&block_env, cfg, None);

        let hash = evm_tx_recovered.hash();

        let evm_db: EvmDb<'_, C> = self.get_db(working_set);

        // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/505
        let result = executor::execute_tx(evm_db, block_env, &evm_tx_recovered, cfg_env).unwrap();

        let from = evm_tx_recovered.signer();
        let to = evm_tx_recovered.to().map(|to| to.into());
        let transaction = reth_rpc_types::Transaction::from_recovered(evm_tx_recovered.tx);

        self.pending_transactions
            .push(&transaction, &mut working_set.accessory_state());

        let receipt = reth_rpc_types::TransactionReceipt {
            transaction_hash: hash.into(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            transaction_index: Some(reth_primitives::U256::from(0)),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            block_hash: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            block_number: Some(reth_primitives::U256::from(0)),
            from,
            to,
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            gas_used: Default::default(),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            cumulative_gas_used: Default::default(),
            contract_address: contract_address(result),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            logs: Default::default(),
            state_root: Some(reth_primitives::U256::from(0).into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            logs_bloom: Default::default(),
            status_code: Some(1u64.into()),
            // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
            effective_gas_price: Default::default(),
            transaction_type: reth_primitives::U8::from(1),
        };

        self.receipts
            .set(&hash.into(), &receipt, &mut working_set.accessory_state());

        Ok(CallResponse::default())
    }
}

/// Get cfg env for a given block number
/// Returns correct config depending on spec for given block number
/// Copies context dependent values from template_cfg or default if not provided
pub(crate) fn get_cfg_env(
    block_env: &BlockEnv,
    cfg: EvmChainCfg,
    template_cfg: Option<CfgEnv>,
) -> CfgEnv {
    CfgEnv {
        chain_id: revm::primitives::U256::from(cfg.chain_id),
        limit_contract_code_size: cfg.limit_contract_code_size,
        spec_id: get_spec_id(cfg.spec, block_env.number).into(),
        // disable_gas_refund: !cfg.gas_refunds, // option disabled for now, we could add if needed
        ..template_cfg.unwrap_or_default()
    }
}

/// Get spec id for a given block number
/// Returns the first spec id defined for block >= block_number
pub(crate) fn get_spec_id(spec: Vec<(u64, SpecIdWrapper)>, block_number: u64) -> SpecIdWrapper {
    match spec.binary_search_by(|&(k, _)| k.cmp(&block_number)) {
        Ok(index) => spec[index].1,
        Err(index) => {
            if index > 0 {
                spec[index - 1].1
            } else {
                // this should never happen as we cover this in genesis
                panic!("EVM spec must start from block 0")
            }
        }
    }
}
