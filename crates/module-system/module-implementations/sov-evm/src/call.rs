#[cfg(feature = "native")]
use std::convert::Infallible;

use crate::conversions::convert_to_transaction_signed;
use crate::db::EvmDb;
use crate::evm::executor::{self};
use crate::evm::primitive_types::{Receipt, TransactionSignedAndRecovered};
use crate::evm::RlpEvmTransaction;
use crate::executor::get_cfg_env;
use crate::{Evm, PendingTransaction, SpecId};
use alloy_primitives::Address;
use reth_primitives::TransactionSigned;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::{serialize, UniversalWallet};
#[cfg(feature = "native")]
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Context, Spec, TxState};

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
        // The signature was checked before the call was dispatched,
        // and the signer was recovered during the authentication process.
        let signer = *context
            .get_sender_credential::<Address>()
            .ok_or(anyhow::anyhow!(
                "EVM transaction must be authenticated by the EVM authenticator"
            ))?;

        // Inside the EVM, we use nonces only for the CREATE operation.
        // The uniqueness check was performed before the call was dispatched.
        let account_nonce = self.get_account_nonce(signer, state)?;

        let evm_tx: TransactionSigned = convert_to_transaction_signed(message.rlp)?;

        let block_env = self
            .block_env
            .get(state)?
            .expect("Pending block must be set");

        let cfg = self.cfg(state)?.expect("Evm config must be set");
        let cfg_env = get_cfg_env(&block_env, cfg, None);

        let evm_db: EvmDb<_, S> = self.get_db(state);

        let result =
            executor::transact_commit(account_nonce, evm_db, &block_env, &evm_tx, signer, cfg_env);

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

                if !is_success {
                    tracing::debug!(
                        hash = hex::encode(evm_tx.hash()),
                        gas_used,
                        ?result,
                        "EVM execution error"
                    );
                    anyhow::bail!("EVM execution error: {:?}", &result);
                }

                tracing::debug!(
                    hash = hex::encode(evm_tx.hash()),
                    gas_used,
                    "EVM transaction has been executed"
                );

                let logs = result.into_logs();

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
            Err(err) => {
                tracing::debug!(
                    tx_hash = hex::encode(evm_tx.hash()),
                    error = ?err,
                    "EVM transaction error"
                );

                anyhow::bail!("EVM transaction error: {:?}", err);
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

        // Fetch `head` and `pending_len` before the `native` code block.
        // This ensures consistent gas charges between native and non-native execution.
        #[allow(unused_variables)]
        let head = self
            .head
            .get(state)?
            .expect("Impossible happened: Head must be set.");

        #[allow(unused_variables)]
        let pending_len = self.pending_transactions.len(state)?;

        #[cfg(feature = "native")]
        self.set_sccessory_state(head, &pending_transaction, pending_len, state)
            .unwrap_infallible();

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
    fn get_account_nonce(
        &self,
        address: Address,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<u64> {
        Ok(self
            .accounts
            .get(&address, state)?
            .map(|acc| acc.nonce)
            .unwrap_or_default())
    }

    #[cfg(feature = "native")]
    fn set_sccessory_state(
        &mut self,
        head: crate::Block,
        pending_transaction: &PendingTransaction,
        pending_tx_len: u64,
        state: &mut impl TxState<S>,
    ) -> Result<(), Infallible> {
        assert!(pending_tx_len > 0);
        let first_tx_index = head.transactions.end;

        let tx_index = first_tx_index
            .checked_add(pending_tx_len)
            .expect("The impossible happened: Tx index overflow.")
            .checked_sub(1)
            //Can't underflow because `pending_tx_len` is greater than 0.
            .expect("The impossible happened: Tx index underflow.");

        self.transactions
            .set(&tx_index, &pending_transaction.transaction, state)?;

        self.receipts
            .set(&tx_index, &pending_transaction.receipt, state)?;

        let hash = pending_transaction.transaction.signed_transaction.hash();
        self.transaction_hashes.set(hash, &tx_index, state)?;

        Ok(())
    }
}

/// Get spec id for a given block number
/// Returns the first spec id defined for block >= block_number
pub(crate) fn get_spec_id(spec: &[(u64, SpecId)], block_number: u64) -> SpecId {
    match spec.binary_search_by_key(&block_number, |&(k, _)| k) {
        Ok(index) => spec[index].1,
        Err(index) => {
            spec[index
                .checked_sub(1)
                .expect("EVM spec must start from block 0")]
            .1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_id_lookup() {
        let spec = vec![(0, SpecId::CONSTANTINOPLE), (2, SpecId::BERLIN)];

        assert_eq!(get_spec_id(&spec, 0), SpecId::CONSTANTINOPLE);
        assert_eq!(get_spec_id(&spec, 1), SpecId::CONSTANTINOPLE);
        assert_eq!(get_spec_id(&spec, 2), SpecId::BERLIN);
        assert_eq!(get_spec_id(&spec, 3), SpecId::BERLIN);
    }
}
