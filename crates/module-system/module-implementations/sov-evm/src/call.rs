use alloy_primitives::Address;
use reth_primitives::TransactionSigned;
use revm::context::result::EVMError;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::{Context, Spec, TxState};

use crate::conversions::convert_to_transaction_signed;
use crate::evm::db::EvmDb;
use crate::evm::executor::{self};
use crate::evm::primitive_types::{Receipt, TransactionSignedAndRecovered};
use crate::evm::RlpEvmTransaction;
use crate::executor::get_cfg_env;
use crate::{Evm, PendingTransaction, SpecId};

/// EVM call message.
#[derive(Debug, PartialEq, Eq, Clone, schemars::JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
pub struct CallMessage {
    /// RLP encoded transaction.
    pub rlp: RlpEvmTransaction,
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    pub(crate) fn execute_call(
        &mut self,
        message: CallMessage,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        // Check if the tx went through the EVM authenticator.
        // TODO: This may no longer be needed.
        //
        // If a sov-modules address is registered as a credential id,
        // then it should be able to send EVM transactions - in that scenario, to give EVM an address, we could
        // use the address of the EVM address that the sov-modules address is registered as a credential to.
        let signer = *context
            .get_sender_credential::<Address>()
            .ok_or(anyhow::anyhow!(
                "EVM transaction must be authenticated by the EVM authenticator"
            ))?;

        let evm_tx: TransactionSigned = convert_to_transaction_signed(message.rlp)?;

        let block_env = self
            .block_env
            .get(state)?
            .expect("Pending block must be set");

        let cfg = self.cfg(state)?.expect("Evm config must be set");
        let cfg_env = get_cfg_env(&block_env, cfg, None);

        let sov_nonce = self.get_sov_nonce(signer, state)?;

        let evm_db: EvmDb<_, S> = self.get_db(state);

        let result = executor::execute_tx(sov_nonce, evm_db, &block_env, &evm_tx, signer, cfg_env);

        let previous_transaction = self.pending_transactions.last(state)?;
        let previous_transaction_cumulative_gas_used = previous_transaction
            .as_ref()
            .map_or(0u64, |tx| tx.receipt.receipt.cumulative_gas_used);
        let log_index_start = previous_transaction.as_ref().map_or(0u64, |tx| {
            tx.receipt
                .log_index_start
                .saturating_add(tx.receipt.receipt.logs.len() as u64)
        });

        let receipt = match result {
            Ok(result) => {
                let is_success = result.is_success();
                let gas_used = result.gas_used();
                let logs = result.into_logs();
                tracing::debug!(
                    hash = hex::encode(evm_tx.hash()),
                    gas_used,
                    "EVM transaction has been executed"
                );
                Receipt {
                    receipt: reth_primitives::Receipt {
                        tx_type: evm_tx.tx_type(),
                        success: is_success,
                        cumulative_gas_used: previous_transaction_cumulative_gas_used
                            .saturating_add(gas_used),
                        logs,
                    },
                    gas_used,
                    log_index_start,
                    error: None,
                }
            }
            // Adopted from https://github.com/paradigmxyz/reth/blob/main/crates/payload/basic/src/lib.rs#L884
            Err(err) => {
                tracing::debug!(
                    tx_hash = hex::encode(evm_tx.hash()),
                    error = ?err,
                    "EVM transaction has been reverted"
                );
                return match err {
                    EVMError::Transaction(_) => {
                        // This is a transactional error, so we can skip it without doing anything.
                        Ok(())
                    }
                    err => {
                        // This is a fatal error, so we need to return it.
                        Err(err.into())
                    }
                };
            }
        };

        let pending_transaction = PendingTransaction {
            transaction: TransactionSignedAndRecovered {
                signer,
                signed_transaction: evm_tx,
                block_number: block_env.number.to::<u64>(),
            },
            receipt,
        };

        self.pending_transactions
            .push(&pending_transaction, state)?;

        Ok(())
    }

    // The nonce check is already performed by the stf-blueprint during transaction preprocessing,
    // so the EVM does not need to perform any additional nonce validation.
    //
    // However, the account nonce is still used by the EVM in the `CREATE` opcode when generating
    // a contract address: `new_address = keccak256(sender, nonce)`.
    // This means we must ensure a unique value is provided to satisfy the opcode.
    // Here, we use the nonce tracked by the EVM, but keep in mind that `eth_getTransactionCount`
    // will return the nonce tracked by the sov-uniqueness module.
    fn get_sov_nonce(&self, address: Address, state: &mut impl TxState<S>) -> anyhow::Result<u64> {
        Ok(self
            .accounts
            .get(&address, state)?
            .map(|acc| acc.nonce)
            .unwrap_or_default())
    }
}

/// Get spec id for a given block number
/// Returns the first spec id defined for block >= block_number
pub(crate) fn get_spec_id(spec: Vec<(u64, SpecId)>, block_number: u64) -> SpecId {
    match spec.binary_search_by(|&(k, _)| k.cmp(&block_number)) {
        Ok(index) => spec[index].1,
        Err(index) => {
            if index > 0 {
                spec[index.checked_sub(1).expect("invalid spec index")].1
            } else {
                // this should never happen as we cover this in genesis
                panic!("EVM spec must start from block 0")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_id_lookup() {
        let spec = vec![
            (0, SpecId::CONSTANTINOPLE),
            (10, SpecId::BERLIN),
            (20, SpecId::LONDON),
            (30, SpecId::CANCUN),
        ];

        assert_eq!(get_spec_id(spec.clone(), 0), SpecId::CONSTANTINOPLE);
        assert_eq!(get_spec_id(spec.clone(), 5), SpecId::CONSTANTINOPLE);
        assert_eq!(get_spec_id(spec.clone(), 10), SpecId::BERLIN);
        assert_eq!(get_spec_id(spec.clone(), 15), SpecId::BERLIN);
        assert_eq!(get_spec_id(spec.clone(), 20), SpecId::LONDON);
        assert_eq!(get_spec_id(spec.clone(), 25), SpecId::LONDON);
        assert_eq!(get_spec_id(spec.clone(), 29), SpecId::LONDON);
        assert_eq!(get_spec_id(spec.clone(), 30), SpecId::CANCUN);
        assert_eq!(get_spec_id(spec.clone(), 35), SpecId::CANCUN);
    }
}
