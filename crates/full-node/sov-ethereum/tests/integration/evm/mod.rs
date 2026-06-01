use sov_evm::{CallMessage, EvmRuntimeConfigUpdate};
use sov_modules_api::transaction::Transaction;
use sov_modules_api::{CryptoSpec, Spec};
use sov_test_utils::default_test_signed_transaction_with_nonce;
use sov_test_utils::test_rollup::TestRollup;

use crate::runtime::{EvmBlueprint, EvmTestSpec, TestRuntime, TestRuntimeCall};

mod evm_account_abstraction;
mod evm_balances;
mod evm_basefee;
mod evm_block_by_number_hash;
mod evm_block_hash;
mod evm_block_number;
mod evm_block_pinned_state_reads;
mod evm_call;
mod evm_call_fee_fields;
mod evm_contract_creation_allowlist;
mod evm_effective_gas_price;
mod evm_fee_history;
mod evm_gas_estimation;
mod evm_get_transaction_count;
mod evm_logs;
mod evm_logs_validation;
mod evm_max_fee_validation;
mod evm_no_gas_limit;
mod evm_oog_error;
mod evm_paymaster_balance_check;
mod evm_publish_reverted_txs;
mod evm_rate_limit;
mod evm_rpc_compliance_validation;
mod evm_rpc_compliance_validation_2;
mod evm_simulation_and_send_consistency;
mod evm_soft_conf;
mod evm_subscribe;
mod evm_timestamp;
mod evm_tracing;
mod evm_tx;
mod evm_tx_type;
mod evm_ws_watch;

/// Signs an EVM `UpdateRuntimeConfig` admin call with `admin_key` and sends it to the sequencer.
///
/// Shared by the EVM integration tests that drive runtime-config changes (e.g. disabling the
/// max-fee check or updating the block gas limit); they differ only in the `update` payload.
async fn send_evm_runtime_config_update(
    rollup: &TestRollup<EvmBlueprint>,
    admin_key: &<<EvmTestSpec as Spec>::CryptoSpec as CryptoSpec>::PrivateKey,
    update: EvmRuntimeConfigUpdate<EvmTestSpec>,
) -> anyhow::Result<()> {
    let msg = TestRuntimeCall::<EvmTestSpec>::Evm(CallMessage::UpdateRuntimeConfig(update));
    let chain_hash =
        <TestRuntime<EvmTestSpec> as sov_modules_stf_blueprint::Runtime<EvmTestSpec>>::CHAIN_HASH;
    let tx: Transaction<TestRuntime<EvmTestSpec>, EvmTestSpec> =
        default_test_signed_transaction_with_nonce(admin_key, &msg, 0, &chain_hash);
    rollup
        .client
        .client
        .send_tx_to_sequencer_with_retry(&tx)
        .await?;
    Ok(())
}
