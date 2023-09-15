use anyhow::Result;
use reth_primitives::TransactionSignedEcRecovered;
use reth_revm::into_reth_log;
use revm::primitives::{CfgEnv, EVMError, SpecId};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::evm::db::EvmDb;
use crate::evm::executor::{self};
use crate::evm::primitive_types::{BlockEnv, Receipt, TransactionSignedAndRecovered};
use crate::evm::{EvmChainConfig, RlpEvmTransaction};
use crate::experimental::PendingTransaction;
use crate::Evm;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize)
)]
#[derive(borsh::BorshDeserialize, borsh::BorshSerialize, Debug, PartialEq, Clone)]
pub struct CallMessage {
    pub tx: RlpEvmTransaction,
}

impl<C: sov_modules_api::Context> Evm<C> {
    pub(crate) fn execute_call(
        &self,
        tx: RlpEvmTransaction,
        _context: &C,
        working_set: &mut WorkingSet<C::Storage>,
    ) -> Result<CallResponse> {
        let evm_tx_recovered: TransactionSignedEcRecovered = tx.try_into()?;
        let block_env = self
            .pending_block
            .get(working_set)
            .expect("Pending block must be set");

        let cfg = self.cfg.get(working_set).expect("Evm config must be set");
        let cfg_env = get_cfg_env(&block_env, cfg, None);

        let evm_db: EvmDb<'_, C> = self.get_db(working_set);
        let result = executor::execute_tx(evm_db, &block_env, &evm_tx_recovered, cfg_env);
        let previous_transaction = self.pending_transactions.last(working_set);
        let previous_transaction_cumulative_gas_used = previous_transaction
            .as_ref()
            .map_or(0u64, |tx| tx.receipt.receipt.cumulative_gas_used);
        let log_index_start = previous_transaction.as_ref().map_or(0u64, |tx| {
            tx.receipt.log_index_start + tx.receipt.receipt.logs.len() as u64
        });

        let receipt = match result {
            Ok(result) => {
                let logs: Vec<_> = result.logs().into_iter().map(into_reth_log).collect();
                let gas_used = result.gas_used();

                Receipt {
                    receipt: reth_primitives::Receipt {
                        tx_type: evm_tx_recovered.tx_type(),
                        success: result.is_success(),
                        cumulative_gas_used: previous_transaction_cumulative_gas_used + gas_used,
                        logs,
                    },
                    gas_used,
                    log_index_start,
                    error: None,
                }
            }
            Err(err) => Receipt {
                receipt: reth_primitives::Receipt {
                    tx_type: evm_tx_recovered.tx_type(),
                    success: false,
                    cumulative_gas_used: previous_transaction_cumulative_gas_used,
                    logs: vec![],
                },
                // TODO: Do we want failed transactions to use all gas?
                gas_used: 0,
                log_index_start,
                error: Some(match err {
                    EVMError::Transaction(err) => EVMError::Transaction(err),
                    EVMError::PrevrandaoNotSet => EVMError::PrevrandaoNotSet,
                    EVMError::Database(_) => EVMError::Database(0u8),
                }),
            },
        };

        let pending_transaction = PendingTransaction {
            transaction: TransactionSignedAndRecovered {
                signer: evm_tx_recovered.signer(),
                signed_transaction: evm_tx_recovered.into(),
                block_number: block_env.number,
            },
            receipt,
        };

        self.pending_transactions
            .push(&pending_transaction, working_set);

        Ok(CallResponse::default())
    }
}

/// Get cfg env for a given block number
/// Returns correct config depending on spec for given block number
/// Copies context dependent values from template_cfg or default if not provided
pub(crate) fn get_cfg_env(
    block_env: &BlockEnv,
    cfg: EvmChainConfig,
    template_cfg: Option<CfgEnv>,
) -> CfgEnv {
    CfgEnv {
        chain_id: revm::primitives::U256::from(cfg.chain_id),
        limit_contract_code_size: cfg.limit_contract_code_size,
        spec_id: get_spec_id(cfg.spec, block_env.number),
        // disable_gas_refund: !cfg.gas_refunds, // option disabled for now, we could add if needed
        ..template_cfg.unwrap_or_default()
    }
}

/// Get spec id for a given block number
/// Returns the first spec id defined for block >= block_number
pub(crate) fn get_spec_id(spec: Vec<(u64, SpecId)>, block_number: u64) -> SpecId {
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
