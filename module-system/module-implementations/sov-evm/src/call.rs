use anyhow::Result;
use ethereum_types::U64;
use reth_primitives::bloom::logs_bloom;
use reth_primitives::{TransactionSignedEcRecovered, U128, U256};
use reth_revm::into_reth_log;
use revm::primitives::{CfgEnv, SpecId};
use sov_modules_api::CallResponse;
use sov_state::WorkingSet;

use crate::evm::db::EvmDb;
use crate::evm::executor::{self};
use crate::evm::transaction::BlockEnv;
use crate::evm::{contract_address, EvmChainConfig, RlpEvmTransaction};
use crate::experimental::PendingTransaction;
use crate::Evm;

#[cfg_attr(
    feature = "native",
    derive(serde::Serialize),
    derive(serde::Deserialize),
    derive(schemars::JsonSchema)
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
        let effective_gas_price =
            U128::from(evm_tx_recovered.effective_gas_price(Some(block_env.basefee)));

        let cfg = self.cfg.get(working_set).expect("Evm config must be set");
        let cfg_env = get_cfg_env(&block_env, cfg, None);

        let evm_db: EvmDb<'_, C> = self.get_db(working_set);
        let result = executor::execute_tx(evm_db, &block_env, &evm_tx_recovered, cfg_env);
        let transaction = reth_rpc_types::Transaction::from_recovered(evm_tx_recovered);

        let receipt = match result {
            Ok(result) => {
                let logs: Vec<_> = result.logs().into_iter().map(into_reth_log).collect();
                let gas_used = U256::from(result.gas_used());

                reth_rpc_types::TransactionReceipt {
                    transaction_hash: Some(transaction.hash),
                    transaction_index: Some(U256::from(
                        self.pending_transactions
                            .len(&mut working_set.accessory_state()),
                    )),
                    block_number: Some(U256::from(block_env.number)),
                    from: transaction.from,
                    to: transaction.to,
                    gas_used: Some(gas_used),
                    // Potentially we can store this in block_env ?
                    cumulative_gas_used: self
                        .pending_transactions
                        .iter(&mut working_set.accessory_state())
                        .map(|tx| tx.receipt.gas_used.unwrap())
                        .sum::<U256>()
                        + gas_used,
                    contract_address: contract_address(&result),
                    status_code: if result.is_success() {
                        Some(U64::from(1))
                    } else {
                        Some(U64::from(0))
                    },
                    // TODO https://github.com/Sovereign-Labs/sovereign-sdk/issues/504
                    effective_gas_price: effective_gas_price,
                    transaction_type: transaction
                        .transaction_type
                        .unwrap()
                        .as_u64()
                        .try_into()
                        .unwrap(),
                    logs_bloom: logs_bloom(logs.iter()),
                    logs: {
                        let mut log_index = 0;
                        logs.into_iter()
                            .map(|log| {
                                let log = reth_rpc_types::Log {
                                    address: log.address,
                                    topics: log.topics,
                                    data: log.data,
                                    // TODO: Those are duplicated data - do we want to store them or calculate on the fly in requests?
                                    // TODO: Maybe we should actually store the primitive types and calculate the rpc types on the fly?
                                    block_hash: transaction.block_hash,
                                    block_number: transaction.block_number,
                                    transaction_hash: Some(transaction.hash),
                                    transaction_index: transaction.transaction_index,
                                    log_index: Some(U256::from(log_index)),
                                    removed: false,
                                };
                                log_index += 1;
                                log
                            })
                            .collect()
                    },
                    block_hash: Default::default(), // Will be filled in end_slot_hook
                    state_root: None, // Pre https://eips.ethereum.org/EIPS/eip-658 (pre-byzantium) and won't be used
                }
            }
            Err(_) => todo!(), // TODO Build failed transaction receipt
        };

        let pending_transaction = PendingTransaction {
            transaction,
            receipt,
        };

        self.pending_transactions
            .push(&pending_transaction, &mut working_set.accessory_state());

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
