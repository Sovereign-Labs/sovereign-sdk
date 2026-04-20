use alloy_consensus::constants::KECCAK_EMPTY;
use alloy_primitives::{Address, B256};
use anyhow::ensure;
use revm::context::result::{EVMError, ExecResultAndState, ExecutionResult};
use revm::context::{BlockEnv, CfgEnv, TxEnv};
use revm::primitives::hardfork::SpecId;
use revm::primitives::HashMap;
use revm::state::Account;
use revm::Database;
use revm_database_interface::DBErrorMarker;
use revm_database_interface::TryDatabaseCommit;
use sov_address::{EthereumAddress, FromVmAddress};
use sov_metrics::{save_elapsed, start_timer};
use sov_modules_api::macros::{serialize, UniversalWallet};
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::{Context, GasSpec, Spec, StateAccessor, TxState};
#[cfg(feature = "native")]
use std::convert::Infallible;

use crate::conversions::{convert_to_tx_signed, create_tx_env};
use crate::db::{self, metrics::MetricsDb};
use crate::evm::primitive_types::{Receipt, TxSignedAndRecovered};
#[cfg(feature = "native")]
use crate::execution_config::EVM_EXECUTION_CONFIG;
use crate::executor::{get_cfg_env, transact};
#[cfg(feature = "native")]
use crate::metrics::EvmTxMetrics;
use crate::sov_fee_and_gas_utils::project_receipt_gas_from_actual_fee;
use crate::{
    gas_metering_mode, BorshSpecId, ChainSpecUpdate, ContractCreationPolicy,
    ContractCreationPolicyUpdate, Evm, EvmChainSpec, EvmRuntimeConfig, EvmRuntimeConfigUpdate,
    GasMeteringMode, PendingTransaction, RlpEvmTransaction, TransactionSigned,
};
use anyhow::{bail, Context as _};

/// The maximum contract code size is 2MB
const MAX_CONTRACT_CODE_SIZE: usize = 2 * 1024 * 1024;
/// Don't let the admin lower the gas limit below 5M to avoid censorship. Setting the gas limit to 0 is censorship.
const MIN_BLOCK_GAS_LIMIT: u64 = 5_000_000;
/// The largest value that the admin can set for the tx gas limit.
/// Setting an infinitely high gas limit would allow DOS by the sequencer/operator.
const MAX_TX_GAS_LIMIT: u64 = 10_000_000_000;

/// EVM call message.
#[derive(Debug, PartialEq, Eq, Clone, schemars::JsonSchema, UniversalWallet)]
#[serialize(Borsh, Serde)]
#[serde(rename_all = "snake_case")]
#[serde(bound = "S: Spec")]
#[schemars(bound = "S: Spec", rename = "call_message")]
pub enum CallMessage<S: Spec> {
    /// RLP encoded transaction.
    Call(RlpEvmTransaction),
    /// Update the runtime configuration
    UpdateRuntimeConfig(EvmRuntimeConfigUpdate<S>),
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
    ) -> anyhow::Result<(
        EvmRuntimeConfig,
        CfgEnv,
        BlockEnv,
        TxEnv,
        TxSignedAndRecovered,
        u64,
    )> {
        let block_env = self.block_env(state)?;

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
        let cfg = self.cfg(state)?;
        let cfg_env = get_cfg_env(&block_env, &cfg, None);
        let gas_limit = self.gas_limit(state, &cfg.chain_spec);
        let tx_env = create_tx_env(&tx, signer, account_nonce, gas_limit);
        let tx = TxSignedAndRecovered::new(signer, tx, block_env.number.to::<u64>());

        Ok((cfg, cfg_env, block_env, tx_env, tx, pending_len))
    }

    pub(crate) fn update_runtime_config(
        &mut self,
        update: EvmRuntimeConfigUpdate<S>,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let Some(admin) = self.admin.get(state)? else {
            bail!("No EVM admin is configured. The config cannot be updated without an admin.");
        };
        ensure!(
            context.sender() == &admin,
            "Only the admin can update the runtime configuration. Got {} but expected {admin}",
            context.sender()
        );
        // Check if all fields are None - this is the signal to disable max fee check
        if update.is_empty() {
            self.disable_max_fee_check.set(&true, state)?;
            return Ok(());
        }

        let mut cfg = self.cfg(state)?;

        let EvmRuntimeConfigUpdate {
            new_hardfork,
            new_contract_creation_policy,
            chain_spec_update,
            new_admin,
        } = update;

        // Update admin (no validation required)
        if let Some(new_admin) = new_admin {
            self.admin.set(&new_admin, state)?;
        }

        // Add hardfork activation, validating that it has a future height and is greater than the current spec id
        if let Some((activation_block_number, BorshSpecId(spec_id))) = new_hardfork {
            self.validate_new_hardfork(activation_block_number, spec_id, &cfg, state)?;
            cfg.hardforks.push((activation_block_number, spec_id));
            cfg.chain_spec
                .hardforks
                .push((activation_block_number, spec_id));
        }

        // Update contract creation policy
        if let Some(new_contract_creation_policy) = new_contract_creation_policy {
            match new_contract_creation_policy {
                ContractCreationPolicyUpdate::Everyone => {
                    cfg.contract_creation_policy = ContractCreationPolicy::Everyone;
                }
                ContractCreationPolicyUpdate::Allowlist { add, remove } => {
                    let mut allowlist = cfg.contract_creation_policy.take_allowlist();
                    for address in add {
                        allowlist.insert(address.0.into());
                    }
                    for address in remove {
                        let address: Address = address.0.into();
                        allowlist.remove(&address);
                    }
                    cfg.contract_creation_policy = ContractCreationPolicy::Allowlist(allowlist);
                }
            }
        }

        // Update the chain spec
        if let Some(chain_spec_update) = chain_spec_update {
            self.apply_chain_spec_update(chain_spec_update, &mut cfg)?;
        }

        self.cfg.set(&cfg, state)?;
        Ok(())
    }

    fn apply_chain_spec_update(
        &mut self,
        chain_spec_update: ChainSpecUpdate,
        cfg: &mut EvmRuntimeConfig,
    ) -> anyhow::Result<()> {
        // Update the contract size limit
        if let Some(new_limit) = chain_spec_update.new_limit_contract_code_size {
            ensure!(
                new_limit < MAX_CONTRACT_CODE_SIZE,
                "Contract code size limit must be less than {MAX_CONTRACT_CODE_SIZE}"
            );
            cfg.chain_spec.limit_contract_code_size = Some(new_limit);
        }

        // Update the block gas limit
        if let Some(new_block_gas_limit) = chain_spec_update.new_block_gas_limit {
            ensure!(
                new_block_gas_limit > MIN_BLOCK_GAS_LIMIT,
                "Block gas limit must be greater than {MIN_BLOCK_GAS_LIMIT} to avoid censorship"
            );
            cfg.chain_spec.block_gas_limit = new_block_gas_limit;
        }

        // Update the tx gas limit
        if let Some(new_tx_gas_limit) = chain_spec_update.new_tx_gas_limit {
            ensure!(new_tx_gas_limit <= cfg.chain_spec.block_gas_limit, "Tx gas limit must be less than or equal to the effective block gas limit after applying the update");
            ensure!(new_tx_gas_limit <= MAX_TX_GAS_LIMIT, "Tx gas limit must be less than or equal to {MAX_TX_GAS_LIMIT} to avoid DOS by the sequencer/operator");
            cfg.chain_spec.tx_gas_limit = Some(new_tx_gas_limit);
        }

        Ok(())
    }

    fn validate_new_hardfork(
        &self,
        activation_block_number: u64,
        spec_id: SpecId,
        cfg: &EvmRuntimeConfig,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        let current_rollup_block = state.rollup_height_to_access().get();
        if current_rollup_block >= activation_block_number {
            bail!("Hardfork activation block number must be greater than the current rollup block. Got {activation_block_number} but expected greater than {current_rollup_block}");
        }

        let (last_hardfork_activation_block, current_spec_id) = cfg
            .hardforks
            .last()
            .cloned()
            .unwrap_or((current_rollup_block, SpecId::CANCUN));

        if spec_id <= current_spec_id {
            bail!("Hardfork spec ID must be greater than the current spec ID. Got {spec_id} but expected greater than {current_spec_id}");
        }

        if activation_block_number <= last_hardfork_activation_block {
            bail!("Hardfork activation block number must be greater than the activation block of the newest hardfork. Got {activation_block_number} but expected greater than {last_hardfork_activation_block}");
        }

        Ok(())
    }

    pub(crate) fn execute_call(
        &mut self,
        message: RlpEvmTransaction,
        context: &Context<S>,
        state: &mut impl TxState<S>,
    ) -> anyhow::Result<()> {
        start_timer!(total);
        // Note: This does *not* verify the signature
        let tx = convert_to_tx_signed(message)?;

        if matches!(tx, alloy_consensus::EthereumTxEnvelope::Eip4844(_)) {
            anyhow::bail!("Eip4844 not supported");
        }

        start_timer!(fetch_state);
        let (cfg, cfg_env, block, tx_env, tx, pending_len) =
            self.fetch_state(context, state, tx)?;

        save_elapsed!(fetch_state_time SINCE fetch_state);
        let db = self.db(state);
        let mut db = MetricsDb::new(db);

        start_timer!(execution);
        let ExecResultAndState {
            result,
            state: state_changes,
        } = match transact(&mut db, &block, tx_env, cfg_env) {
            Ok(result) => result,
            Err(err) => return on_error(*tx.signed_transaction.hash(), err),
        };

        // Subtract the gas balance from the caller's account here. If balance is subzero, revert the SDK transaction
        save_elapsed!(execution_time SINCE execution);
        verify_contract_creation_allowlist(&state_changes, &tx.signer, &cfg, &mut db)?;
        #[cfg(feature = "native")]
        let new_pinned_contracts = get_pinned_contract_list_updates(
            &state_changes,
            &tx.signer,
            &mut db,
        )
        .inspect_err(|err| tracing::debug!(error = ?err, "Ran out of gas while getting pinned contract list updates"))
        .map_err(|err| anyhow::anyhow!("EVM transaction error: {err:?}"))?;

        // We don't use transact_commit as it does not support returning an error
        start_timer!(state_commit);
        db.try_commit(state_changes)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        save_elapsed!(state_commit_time SINCE state_commit);

        if !result.is_success() {
            on_revert(*tx.signed_transaction.hash(), &result, context)?;
        }
        #[cfg(feature = "native")]
        let db_metrics = db.metrics();

        let gas_used = result.gas_used();
        start_timer!(receipt_t);
        let receipt = self.create_receipt(&tx, pending_len, result, state)?;
        save_elapsed!(receipt_time SINCE receipt_t);
        state.charge_linear_gas(<S as GasSpec>::gas_to_charge_per_evm_gas(), gas_used as u32)?;

        start_timer!(set_state);

        // Note that we get the time unconditionally here, as we want to store the time in the pending transaction and have consistent gas metering across zk/native
        let time = self.chain_state_module.get_oracle_time(state)?;

        let mut pending_tx = PendingTransaction::new(tx, receipt, time);
        self.pending_transactions.push(&pending_tx, state)?;
        save_elapsed!(set_state_time SINCE set_state);

        start_timer!(get_head_t);
        // Fetch `head` and `pending_len` before the `native` code block.
        // This ensures consistent gas charges between native and non-native execution.
        #[allow(unused_variables)]
        let head = self
            .head
            .get(state)?
            .expect("Head is set in genesis and never deleted");
        save_elapsed!(get_head_time SINCE get_head_t);

        // Capture fee after metered state updates are complete so receipts reconcile against
        // the same charged amount users observe in balance deltas.
        let gas_info = state
            .try_as_basic_gas_meter()
            .expect("TxState should have BasicGasMeter")
            .gas_info();

        if let Some(projected_gas) =
            project_receipt_gas_from_actual_fee::<S>(&pending_tx.receipt, &gas_info)?
        {
            pending_tx.receipt.gas_used = projected_gas.gas_used;
            pending_tx.receipt.receipt.cumulative_gas_used = projected_gas.cumulative_gas_used;

            // The pending tx is already pushed in metered mode and contributes to charged fee.
            // This follow-up write only synchronizes receipt fields with that finalized fee.
            let mut unmetered_state = state.to_unmetered();
            let set_result = self
                .pending_transactions
                .set(pending_len, &pending_tx, &mut unmetered_state)
                .unwrap_infallible();
            set_result.map_err(|err| {
                anyhow::anyhow!("EVM: failed to update pending projected receipt: {err}")
            })?;
        }

        #[cfg(feature = "native")]
        let set_accessory_state_time = {
            start_timer!(set_accessory_state);
            let tx_fee_paid = gas_info.gas_value;
            // Since we just inserted tx above, we need to increment `pending_len`` by 1.
            self.set_accessory_state(head, &pending_tx, pending_len + 1, tx_fee_paid, state)
                .unwrap_infallible();
            set_accessory_state.elapsed()
        };

        // Now that we've saved the transaction, we can update the pinned contract list.
        // Worst case scenario, the transaction might still revert in the post-hook; then we'll end up with an extra bucket in the pinned cache,
        // but that bucket will be empty so the waste isn't big and there's no correctness issue (because pinned buckets are just a mirror of the DB anyway).
        // We can always clean it up manually later.
        #[cfg(feature = "native")]
        self.update_pinned_contract_list(&new_pinned_contracts, state);

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

    fn gas_limit(&self, state: &mut impl TxState<S>, spec: &EvmChainSpec) -> u64 {
        let gas_meter = state
            .try_as_basic_gas_meter()
            .expect("TxState should have BasicGasMeter");
        let funds = gas_meter.remaining_funds.map(|funds| funds.0).unwrap_or(0);
        let gas = gas_meter.remaining_gas.as_ref()[0];
        let price = gas_meter.gas_price.as_ref()[0].0;
        let gas_limit = match (funds, gas) {
            (0, 0) => 0,
            (_, 0) => u64::MAX,
            (funds, gas) => {
                let gas_from_funds = (funds.checked_div(price).unwrap_or(u64::MAX as u128))
                    .min(u64::MAX as u128) as u64;
                gas.min(gas_from_funds)
            }
        };
        gas_limit.min(spec.tx_gas_limit.unwrap_or(u64::MAX))
    }

    fn sequencer_gas_used(&self, state: &mut impl TxState<S>) -> u64 {
        let gas_meter = state
            .try_as_basic_gas_meter()
            .expect("TxState should have BasicGasMeter");
        let sequencer_gas_used =
            gas_meter.initial_gas.as_ref()[0] - gas_meter.remaining_gas.as_ref()[0];
        let evm_gas_to_sequencer_gas_ratio =
            <S as GasSpec>::gas_to_charge_per_evm_gas().as_ref()[0];
        sequencer_gas_used
            .checked_div(evm_gas_to_sequencer_gas_ratio)
            .expect("gas_to_charge_per_evm_gas() should not be zero")
    }

    pub(crate) fn create_receipt(
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
                .expect("We should never have more than u64::MAX logs")
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

        let receipt = crate::evm::eth_receipt::EthReceipt {
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
    pub(crate) fn set_accessory_state(
        &mut self,
        head: crate::Block,
        pending_transaction: &PendingTransaction,
        pending_tx_len: u64,
        tx_fee_paid: sov_bank::Amount,
        state: &mut impl TxState<S>,
    ) -> Result<(), Infallible> {
        assert!(pending_tx_len > 0);
        let first_tx_index = head.transactions.end;

        let tx_index = first_tx_index
            .checked_add(pending_tx_len - 1)
            .expect("We should never have more than u64::MAX transactions");

        self.transactions
            .set(&tx_index, &pending_transaction.transaction, state)?;

        self.receipts.set(
            &tx_index,
            &(
                pending_transaction.receipt.clone(),
                pending_transaction.time.clone(),
            ),
            state,
        )?;
        self.receipt_fees.set(&tx_index, &tx_fee_paid, state)?;

        let hash = pending_transaction.transaction.signed_transaction.hash();
        self.transaction_hashes.set(hash, &tx_index, state)?;

        Ok(())
    }
}

pub(crate) fn verify_contract_creation_allowlist<
    DB: Database<Error = E>,
    E: DBErrorMarker + std::fmt::Display,
>(
    state_changes: &HashMap<Address, Account>,
    signer: &Address,
    cfg: &EvmRuntimeConfig,
    db: &mut DB,
) -> anyhow::Result<()> {
    if cfg.contract_creation_policy.allows(signer) {
        return Ok(());
    }
    for (address, account) in state_changes.iter() {
        let is_contract = account
            .info
            .code
            .as_ref()
            .is_some_and(|code| !code.is_empty());
        if is_contract {
            let was_not_contract = db
                .basic(*address)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "Error while fetching previous contract data to verify allowlist: {e}"
                    )
                })?
                .map(|acc| acc.code_hash == KECCAK_EMPTY)
                .unwrap_or(true);
            // If it wasn't a contract before, and it is now, it was just deployed. Pin it if necessary.
            if was_not_contract {
                bail!("Contract creation is only allowed from allowed addresses. {signer} is not on the list");
            }
        }
    }
    Ok(())
}

#[cfg(feature = "native")]
/// Get the list of new contracts to pin from the state changes.
pub(crate) fn get_pinned_contract_list_updates<DB: Database<Error = E>, E: DBErrorMarker>(
    state_changes: &HashMap<Address, Account>,
    signer: &Address,
    db: &mut DB,
) -> Result<Vec<Address>, E> {
    let Some(execution_config) = EVM_EXECUTION_CONFIG.get() else {
        return Ok(Vec::new());
    };

    let execution_config = execution_config
        .read()
        .expect("EVM Execution config RW lock is poisoned.");
    if !execution_config
        .contents
        .privileged_deployer_addresses
        .contains(signer)
    {
        return Ok(Vec::new());
    };

    let mut new_pinned_contracts = Vec::new();
    for (address, account) in state_changes.iter() {
        let is_contract = account
            .info
            .code
            .as_ref()
            .is_some_and(|code| !code.is_empty());
        if is_contract {
            let was_not_contract = db
                .basic(*address)?
                .map(|acc| acc.code_hash == KECCAK_EMPTY)
                .unwrap_or(true);
            // If it wasn't a contract before, and it is now, it was just deployed. Pin it if necessary.
            if was_not_contract {
                new_pinned_contracts.push(*address);
            }
        }
    }
    Ok(new_pinned_contracts)
}

#[cfg(feature = "native")]
impl<S: Spec> Evm<S> {
    pub(crate) fn update_pinned_contract_list(
        &self,
        new_pinned_contracts: &[Address],
        state: &mut impl TxState<S>,
    ) {
        use crate::execution_config::EVM_EXECUTION_CONFIG;
        // If there are no new pinned contracts, we can return early.
        if new_pinned_contracts.is_empty() {
            return;
        }
        // Similarly, if the execution config is not initialized, we can return early.
        let Some(execution_config) = EVM_EXECUTION_CONFIG.get() else {
            return;
        };

        // Now for each new contract we need to track, add it to the execution config and load the bucket into the pinned cache.
        let mut execution_config = execution_config
            .write()
            .expect("EVM Execution config RW lock is poisoned.");
        let size_limit = execution_config.contents.default_bucket_size_limit;
        let storage = state.storage().clone();
        let mut pinned_cache = state.pinned_cache_mut();
        let mut updated_execution_config = false;
        for address in new_pinned_contracts {
            // Refresh the storage for this accessor, if necessary.
            if let Some(pinned_cache) = pinned_cache.as_mut() {
                use sov_state::pinned_cache::LoadBucketOutcome;

                let bucket_id = self.get_bucket_id_for_address(address);
                match pinned_cache.try_load_bucket_if_absent(bucket_id, &storage, size_limit) {
                    Err(e) => {
                        tracing::warn!(address = ?address, error = ?e, "EVM Failed to load bucket for address into pinned cache");
                    }
                    Ok(LoadBucketOutcome::Loaded) => {
                        tracing::debug!(address = ?address, "EVM Loaded bucket for address into pinned cache");
                    }
                    Ok(LoadBucketOutcome::OverSizeLimit) => {
                        tracing::warn!(address = ?address, "EVM Failed to load bucket for address into pinned cache because it exceeded the size limit");
                    }
                    Ok(LoadBucketOutcome::AlreadyPresent) => {
                        tracing::debug!(address = ?address, "EVM didn't load for address into pinned cache because it is already present in the cache");
                    }
                    Ok(LoadBucketOutcome::NotSupportedByStorage) => {
                        tracing::error!(address = ?address, "EVM Failed to load bucket for address into pinned cache because the storage doesn't support iteration. This means that pinning is configured but the rollup doesnt support it.");
                    }
                }
            }
            // Update the execution config with the new address to track. If the address didn't already exist, mark it as dirty so that we flush to disk after.
            if execution_config
                .contents
                .known_contracts_and_limits
                .insert(*address, size_limit)
                .is_none()
            {
                updated_execution_config = true;
            }
        }
        if updated_execution_config {
            tracing::debug!("EVM Execution config updated, flushing to disk");
            std::fs::write(&execution_config.location, serde_json::to_string_pretty(&execution_config.contents).expect("Failed to serialize execution config")).unwrap_or_else(|e| panic!("EVM Failed to write execution config to file {}: {}. This usually means that the file permissions changed while the rollup was running.", execution_config.location.display(), e));
        }
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

fn on_revert<S: Spec>(
    hash: B256,
    result: &ExecutionResult,
    context: &Context<S>,
) -> Result<(), anyhow::Error> {
    #[cfg(feature = "native")]
    let preferred_sequencer_publish_reverted_txs = EVM_EXECUTION_CONFIG
        .get()
        .map(|conf| {
            conf.read()
                .expect("Mutex must not be poisoned")
                .contents
                .preferred_sequencer_publish_reverted_txs
        })
        .unwrap_or(false);
    #[cfg(not(feature = "native"))]
    let preferred_sequencer_publish_reverted_txs = false;
    tracing::debug!(
        hash = hex::encode(hash),
        gas_used = result.gas_used(),
        ?result,
        publish = %preferred_sequencer_publish_reverted_txs,
        "EVM execution error"
    );
    // Revert the sovereign SDK transaction only if
    // 1. We're in the sequencer
    // 2. The submitter of this transaction is the preferred sequencer
    // 3. The preferred sequencer is not configured to publish reverted transactions
    //
    // Reverting the tx *in the preferred sequencer* will cause it to be rejected and excluded from the batch. Reverting it in any other context
    // will simply cause it to be excluded from the EVM's record keeping.
    if context.execution_context().is_sequencer()
        && context.sequencer_is_preferred()
        && !preferred_sequencer_publish_reverted_txs
    {
        anyhow::bail!("EVM execution error: {:?}", result);
    }

    Ok(())
}

/// Get spec id for a given block number
/// Returns the first spec id defined for block >= block_number
pub(crate) fn get_spec_id(spec: &[(u64, SpecId)], block_number: u64) -> SpecId {
    let index = match spec.binary_search_by_key(&block_number, |&(k, _)| k) {
        Ok(index) => index,
        Err(index) => index
            .checked_sub(1)
            .expect("EVM spec must start from block 0"),
    };
    spec[index].1
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
