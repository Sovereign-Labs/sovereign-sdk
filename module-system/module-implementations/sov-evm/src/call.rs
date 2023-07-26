use anyhow::Result;
use revm::primitives::CfgEnv;
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::evm::contract_address;
use crate::evm::db::EvmDb;
use crate::evm::executor::{self};
use crate::evm::transaction::EvmTransaction;
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
        // https://github.com/Sovereign-Labs/sovereign-sdk/issues/516
        let cfg_env = CfgEnv::default();
        let block_env = self.block_env.get(working_set).unwrap_or_default();
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
}
