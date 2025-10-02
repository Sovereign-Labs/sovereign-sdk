use alloy_primitives::{Address, B256};
use reth_primitives::TransactionSigned;
use revm::context::result::{EVMError, ExecResultAndState, ExecutionResult};
use revm::context::{BlockEnv, CfgEnv, TxEnv};
use revm::primitives::hardfork::SpecId;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_metrics::{save_elapsed, start_timer};
use sov_modules_api::macros::{serialize, UniversalWallet};
#[cfg(feature = "native")]
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Context, GasSpec, Spec, TxState};
#[cfg(feature = "native")]
use std::convert::Infallible;

use crate::conversions::{convert_to_tx_signed, create_tx_env};
use crate::db::{self, commit::FallibleDatabaseCommit, metrics::MetricsDb};
use crate::evm::primitive_types::{Receipt, TxSignedAndRecovered};
use crate::evm::RlpEvmTransaction;
use crate::executor::{get_cfg_env, transact};
#[cfg(feature = "native")]
use crate::metrics::EvmTxMetrics;
use crate::{gas_metering_mode, Evm, GasMeteringMode, PendingTransaction};
use anyhow::Context as _;

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
    pub(crate) fn fetch_state(
        &mut self,
        context: &Context<S>,
        state: &mut impl TxState<S>,
        tx: TransactionSigned,
    ) -> anyhow::Result<(CfgEnv, BlockEnv, TxEnv, TxSignedAndRecovered, u64)> {
        let block_env = self.block_env(state)?.expect(
            "The impossible happened: block_env should be set in `begin_rollup_block_hook`.",
        );

        // The signature was checked before the call was dispatched,
        // and the signer was recovered during the authentication process.
        let signer = *context
            .get_sender_credential::<Address>()
            .ok_or(anyhow::anyhow!(
                "EVM transaction must be authenticated by the EVM authenticator"
            ))?;

        let pending_len = self.pending_transactions.len(state)?;

        // Inside the EVM, we use nonces only for the CREATE operation.
        // The uniqueness check was performed before the call was dispatched.
        let account_nonce = self.get_account_nonce(signer, state)?;
        let gas_limit = self.gas_limit(state);
        let tx_env = create_tx_env(&tx, signer, account_nonce, gas_limit);
        let tx = TxSignedAndRecovered::new(signer, tx, block_env.number.to::<u64>());
        let cfg = self.cfg(state)?;
        let cfg_env = get_cfg_env(&block_env, cfg, None);

        Ok((cfg_env, block_env, tx_env, tx, pending_len))
    }

    pub(crate) fn execute_call(
        &mut self,
        message: CallMessage,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        start_timer!(total);
        let tx = convert_to_tx_signed(message.rlp)?;

        if matches!(tx, alloy_consensus::EthereumTxEnvelope::Eip4844(_)) {
            anyhow::bail!("Eip4844 not supported");
        }

        start_timer!(fetch_state);
        let (cfg, block, tx_env, tx, pending_len) = self.fetch_state(context, state, tx)?;
        save_elapsed!(fetch_state_time SINCE fetch_state);
        let db = self.get_db(state);
        let mut db = MetricsDb::new(db);

        start_timer!(execution);
        let ExecResultAndState {
            result,
            state: state_changes,
        } = match transact(&mut db, &block, tx_env, cfg) {
            Ok(result) => result,
            Err(err) => return on_error(*tx.signed_transaction.hash(), err),
        };
        save_elapsed!(execution_time SINCE execution);
        // We don't use transact_commit as it does not support returning an error
        start_timer!(state_commit);
        db.commit(state_changes)
            .map_err(|e| anyhow::anyhow!("{}", &*e))?;
        save_elapsed!(state_commit_time SINCE state_commit);

        if !result.is_success() {
            return on_revert(*tx.signed_transaction.hash(), result);
        }
        #[cfg(feature = "native")]
        let db_metrics = db.metrics();
        drop(db); // To release the state

        let gas_used = result.gas_used();
        start_timer!(receipt_t);
        let receipt = self.get_receipt(&tx, pending_len, result, state)?;
        save_elapsed!(receipt_time SINCE receipt_t);
        state.charge_linear_gas(
            &<S as GasSpec>::gas_to_charge_per_evm_gas(),
            gas_used as u32,
        )?;

        start_timer!(set_state);
        let pending_tx = PendingTransaction::new(tx, receipt);
        self.pending_transactions.push(&pending_tx, state)?;
        save_elapsed!(set_state_time SINCE set_state);

        start_timer!(get_head_t);
        // Fetch `head` and `pending_len` before the `native` code block.
        // This ensures consistent gas charges between native and non-native execution.
        #[allow(unused_variables)]
        let head = self
            .head
            .get(state)?
            // Justified, we set it at `genesis` and leter only override it.
            .expect("Impossible happened: Head must be set.");
        save_elapsed!(get_head_time SINCE get_head_t);

        #[cfg(feature = "native")]
        let set_accessory_state_time = {
            start_timer!(set_accessory_state);
            // Since we just inserted tx above, we need to increment `pending_len`` by 1.
            self.set_accessory_state(head, &pending_tx, pending_len + 1, state)
                .unwrap_infallible();
            set_accessory_state.elapsed()
        };

        save_elapsed!(total_time SINCE total);
        #[cfg(feature = "native")]
        {
            let metrics = EvmTxMetrics {
                total_time,
                fetch_state_time,
                execution_time,
                state_commit_time,
                receipt_time,
                set_state_time,
                get_head_time,
                set_accessory_state_time,
            };
            sov_metrics::track_metrics(|t| {
                t.submit(metrics);
            });
            sov_metrics::track_metrics(|t| {
                t.submit(db_metrics);
            });
        }

        Ok(())
    }

    fn gas_limit(&self, state: &mut impl TxState<S>) -> u64 {
        let gas_meter = state
            .try_as_basic_gas_meter()
            // Justified, `impl TxState` has access to `BasicGasState`.
            .expect("The impossible happened: BasicGasState is absent.");
        let funds = gas_meter
            .remaining_funds
            .expect("This method is used in the context where the amount is set")
            .0;
        let gas = gas_meter.remaining_gas.as_ref()[0];
        let price = gas_meter.gas_price.as_ref()[0].0;
        match (funds, gas) {
            (0, 0) => 0,
            (_, 0) => u64::MAX,
            (funds, gas) => {
                let gas_from_funds = (funds / price).min(u64::MAX as u128) as u64;
                gas.min(gas_from_funds)
            }
        }
    }

    fn sequencer_gas_used(&self, state: &mut impl TxState<S>) -> u64 {
        let gas_meter = state.try_as_basic_gas_meter().unwrap();
        let sequencer_gas_used =
            gas_meter.initial_gas.as_ref()[0] - gas_meter.remaining_gas.as_ref()[0];
        let evm_gas_to_sequencer_gas_ratio =
            <S as GasSpec>::gas_to_charge_per_evm_gas().as_ref()[0];
        sequencer_gas_used
            .checked_div(evm_gas_to_sequencer_gas_ratio)
            .expect("gas_to_charge_per_evm_gas() is zero")
    }

    fn get_receipt(
        &self,
        tx: &TxSignedAndRecovered,
        tx_index: u64,
        result: ExecutionResult,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<Receipt> {
        let previous_transaction = self.pending_transactions.last(state)?;
        let previous_transaction_cumulative_gas_used = previous_transaction
            .as_ref()
            .map_or(0u64, |tx| tx.receipt.receipt.cumulative_gas_used);

        let log_index_start = previous_transaction.as_ref().map_or(0u64, |tx| {
            tx.receipt
                .log_index_start
                .checked_add(tx.receipt.receipt.logs.len() as u64)
                // Justified, we will never have that many logs.
                .expect("Impossible happened: Log index overflow.")
        });
        let is_success = result.is_success();
        let gas_used = result.gas_used()
            + match gas_metering_mode() {
                GasMeteringMode::Rollup => self.sequencer_gas_used(state),
                GasMeteringMode::Evm => 0,
            };
        let logs = result.into_logs();
        let transaction_hash = *tx.signed_transaction.hash();
        tracing::debug!(
            hash = hex::encode(transaction_hash),
            gas_used,
            "EVM transaction has been executed"
        );

        let receipt = reth_primitives::Receipt {
            tx_type: tx.signed_transaction.tx_type(),
            success: is_success,
            cumulative_gas_used: previous_transaction_cumulative_gas_used
                .checked_add(gas_used)
                .context("EVM: Cumulative gas used overflow")?,

            logs,
        };

        Ok(Receipt {
            receipt,
            transaction_hash,
            block_number: tx.block_number,
            gas_used,
            log_index_start,
            error: None,
            transaction_index: tx_index,
        })
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
    fn set_accessory_state(
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
            // Justified, can't underflow because `pending_tx_len` is greater than 0.
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

fn on_error<S: Spec>(
    hash: B256,
    err: EVMError<db::Error<impl TxState<S>>>,
) -> Result<(), anyhow::Error> {
    tracing::debug!(
        tx_hash = hex::encode(hash),
        error = ?err,
        "EVM transaction error"
    );
    anyhow::bail!("EVM transaction error: {:?}", err);
}

fn on_revert(hash: B256, result: ExecutionResult) -> Result<(), anyhow::Error> {
    tracing::debug!(
        hash = hex::encode(hash),
        gas_used = result.gas_used(),
        ?result,
        "EVM execution error"
    );
    anyhow::bail!("EVM execution error: {:?}", &result);
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
