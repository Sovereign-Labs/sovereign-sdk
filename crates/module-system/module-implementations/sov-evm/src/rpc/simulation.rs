use std::ops::DerefMut;

use alloy_eips::BlockId;
use alloy_primitives::{Address, U256};
use alloy_rpc_types::{
    state::{AccountOverride, StateOverride},
    BlockOverrides, TransactionRequest,
};
use revm::context::result::ResultAndState;
use revm::context::{BlockEnv, CfgEnv};
use revm::database::State as RevmState;
use revm::primitives::HashMap as RevmHashMap;
use revm::state::{Account, AccountStatus, Bytecode, EvmStorageSlot};
use revm::{Database, DatabaseCommit};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::config_value;
use sov_modules_api::{ApiStateAccessor, Spec};
use sov_rpc_eth_types::{invalid_params_rpc_err, EthApiError, RpcInvalidTransactionError};

use super::maybe_archival_state::MaybeArchivalState;
use crate::db::EvmDb;
use crate::error::into_rpc_error;
use crate::evm::executor;
use crate::executor::get_cfg_env;
use crate::helpers::prepare_call_env;
use crate::{verify_contract_creation_allowlist, Evm};

/// Validates fee-field consistency in an `eth_call` / `eth_estimateGas` /
/// `eth_createAccessList` request.
///
/// These are request-format checks independent of the caller's balance.
/// Balance-based upfront cost validation lives in `sov-ethereum`'s RPC
/// handler where full paymaster context is available.
pub(crate) fn validate_call_fee_fields(request: &TransactionRequest) -> Result<(), EthApiError> {
    if request.gas_price.is_some()
        && (request.max_fee_per_gas.is_some() || request.max_priority_fee_per_gas.is_some())
    {
        return Err(EthApiError::ConflictingFeeFieldsInRequest);
    }

    if let (Some(max_fee_per_gas), Some(max_priority_fee_per_gas)) =
        (request.max_fee_per_gas, request.max_priority_fee_per_gas)
    {
        if max_priority_fee_per_gas > max_fee_per_gas {
            return Err(RpcInvalidTransactionError::TipAboveFeeCap.into());
        }
    }

    Ok(())
}

pub(crate) fn validate_simulation_max_fee_against_base_fee(
    request: &TransactionRequest,
    block_env: &BlockEnv,
    enforce_max_fee_check: bool,
) -> Result<(), EthApiError> {
    // The simulated call context still executes with zero revm fees, but `eth_call` and
    // `eth_estimateGas` must reject EIP-1559 requests that real tx admission would reject once
    // the shared gate is active for the selected state.
    if enforce_max_fee_check {
        if let Some(max_fee_per_gas) = request.max_fee_per_gas {
            if max_fee_per_gas < u128::from(block_env.basefee) {
                return Err(RpcInvalidTransactionError::FeeCapTooLow.into());
            }
        }
    }

    Ok(())
}

impl<S: Spec> Evm<S>
where
    S::Address: FromVmAddress<EthereumAddress>,
{
    pub(crate) fn resolve_simulation_nonce(
        &self,
        request: &mut TransactionRequest,
        state: &mut MaybeArchivalState<'_, S>,
    ) -> Result<(), EthApiError> {
        // Keep omitted nonce behavior aligned with the caller's current account nonce,
        // but only after fee-cap validation so mixed low-fee + stale-nonce requests
        // surface the same error precedence as raw transaction submission.
        if let Some(from) = request.from {
            let credential_id = EthereumAddress::from(from).as_credential_id();
            let account_nonce = self
                .uniqueness_module
                .next_nonce(&credential_id, state.deref_mut())
                .map_err(|e| EthApiError::other(into_rpc_error(e)))?;

            match request.nonce {
                Some(tx_nonce) if tx_nonce < account_nonce => {
                    return Err(RpcInvalidTransactionError::NonceTooLow {
                        tx: tx_nonce,
                        state: account_nonce,
                    }
                    .into());
                }
                None => request.nonce = Some(account_nonce),
                _ => {}
            }
        }

        Ok(())
    }

    pub(crate) fn resolve_simulation_context_for_block_id<'a>(
        &self,
        block_id: Option<BlockId>,
        state: &'a mut ApiStateAccessor<S>,
    ) -> Result<
        (
            BlockEnv,
            MaybeArchivalState<'a, S>,
            crate::config::EvmRuntimeConfig,
        ),
        EthApiError,
    > {
        let block_env = self.resolve_block_env_for_call(block_id, state)?;
        let mut maybe_archival_state = self.resolve_state_for_block_id(block_id, state)?;
        // Omitted-gas simulations must read the tx gas cap from the selected state so
        // archival queries remain stable after later runtime-config updates.
        let cfg = self.cfg_infallible(maybe_archival_state.deref_mut());
        Ok((block_env, maybe_archival_state, cfg))
    }

    pub(crate) fn call(
        &self,
        request: TransactionRequest,
        block_id: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
        state: &mut ApiStateAccessor<S>,
    ) -> Result<ResultAndState, EthApiError> {
        let (block_env, maybe_archival_state, cfg) =
            self.resolve_simulation_context_for_block_id(block_id, state)?;
        self.call_with_context(
            request,
            block_env,
            maybe_archival_state,
            &cfg,
            state_overrides,
            block_overrides,
        )
    }

    pub(crate) fn call_with_context(
        &self,
        mut request: TransactionRequest,
        mut block_env: BlockEnv,
        mut maybe_archival_state: MaybeArchivalState<'_, S>,
        cfg: &crate::config::EvmRuntimeConfig,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> Result<ResultAndState, EthApiError> {
        let has_overrides = state_overrides.is_some() || block_overrides.is_some();
        let enforce_max_fee_check = self
            .is_max_fee_check_active(maybe_archival_state.deref_mut())
            .map_err(|e| EthApiError::other(into_rpc_error(e)))?;
        validate_call_fee_fields(&request)?;

        // Fee validation: with overrides, validate against the overridden block_env;
        // without overrides, validate against the original block_env.
        if has_overrides {
            let mut validation_block_env = block_env.clone();
            {
                let evm_db: EvmDb<_, S> = self.db(maybe_archival_state.deref_mut());
                let mut validation_state = RevmState::builder().with_database(evm_db).build();
                apply_call_overrides(
                    &mut validation_state,
                    &mut validation_block_env,
                    state_overrides.clone(),
                    block_overrides.clone(),
                )?;
            }
            validate_simulation_max_fee_against_base_fee(
                &request,
                &validation_block_env,
                enforce_max_fee_check,
            )?;
        } else {
            validate_simulation_max_fee_against_base_fee(
                &request,
                &block_env,
                enforce_max_fee_check,
            )?;
        }
        self.resolve_simulation_nonce(&mut request, &mut maybe_archival_state)?;

        if !has_overrides {
            let mut evm_db: EvmDb<_, S> = self.db(maybe_archival_state.deref_mut());
            return simulate_call(&mut evm_db, &block_env, cfg, request);
        }

        let evm_db: EvmDb<_, S> = self.db(maybe_archival_state.deref_mut());
        let mut evm_state = RevmState::builder().with_database(evm_db).build();
        apply_call_overrides(
            &mut evm_state,
            &mut block_env,
            state_overrides,
            block_overrides,
        )?;
        simulate_call(&mut evm_state, &block_env, cfg, request)
    }
}

fn invalid_override_params(message: impl Into<String>) -> EthApiError {
    EthApiError::other(invalid_params_rpc_err(message.into()))
}

/// Executes the shared tail of a simulation call: builds the EVM config, prepares
/// the transaction environment, runs the transaction, and verifies the allowlist.
fn simulate_call<DB: Database>(
    db: &mut DB,
    block_env: &BlockEnv,
    cfg: &crate::config::EvmRuntimeConfig,
    request: TransactionRequest,
) -> Result<ResultAndState, EthApiError>
where
    DB::Error: Into<EthApiError> + revm_database_interface::DBErrorMarker + std::fmt::Display,
{
    let cfg_env = get_cfg_env(block_env, cfg, Some(get_cfg_env_template()));
    let tx_env = prepare_call_env(block_env, request, cfg.chain_spec.tx_gas_limit)?;
    let caller = tx_env.caller;
    let result = executor::transact(&mut *db, block_env, tx_env, cfg_env)?;
    verify_contract_creation_allowlist(&result.state, &caller, cfg, db).map_err(|e| {
        EthApiError::other(sov_rpc_eth_types::rpc_error_with_code(
            alloy_rpc_types::error::EthRpcErrorCode::TransactionRejected.code(),
            e.to_string(),
        ))
    })?;
    Ok(result)
}

pub(crate) fn apply_call_overrides<DB: Database>(
    db: &mut RevmState<DB>,
    block_env: &mut BlockEnv,
    state_overrides: Option<StateOverride>,
    block_overrides: Option<Box<BlockOverrides>>,
) -> Result<(), EthApiError>
where
    DB::Error: Into<EthApiError>,
{
    if let Some(state_overrides) = state_overrides {
        apply_state_overrides(db, state_overrides)?;
    }
    if let Some(block_overrides) = block_overrides {
        apply_block_overrides(db, block_env, *block_overrides)?;
    }
    Ok(())
}

fn apply_block_overrides<DB: Database>(
    db: &mut RevmState<DB>,
    block_env: &mut BlockEnv,
    block_overrides: BlockOverrides,
) -> Result<(), EthApiError> {
    let BlockOverrides {
        number,
        difficulty,
        time,
        gas_limit,
        coinbase,
        random,
        base_fee,
        block_hash,
        ..
    } = block_overrides;

    if let Some(block_hash) = block_hash {
        db.block_hashes.extend(block_hash);
    }
    if let Some(number) = number {
        let block_number = u64::try_from(number).map_err(|_| {
            invalid_override_params(format!("block number overflow: {number} exceeds u64::MAX"))
        })?;
        block_env.number = U256::from(block_number);
    }
    if let Some(difficulty) = difficulty {
        block_env.difficulty = difficulty;
    }
    if let Some(time) = time {
        block_env.timestamp = U256::from(time);
    }
    if let Some(gas_limit) = gas_limit {
        block_env.gas_limit = gas_limit;
    }
    if let Some(coinbase) = coinbase {
        block_env.beneficiary = coinbase;
    }
    if let Some(random) = random {
        block_env.prevrandao = Some(random);
    }
    if let Some(base_fee) = base_fee {
        block_env.basefee = u64::try_from(base_fee).map_err(|_| {
            invalid_override_params(format!("base fee overflow: {base_fee} exceeds u64::MAX"))
        })?;
    }
    Ok(())
}

fn apply_state_overrides<DB: Database>(
    db: &mut RevmState<DB>,
    state_overrides: StateOverride,
) -> Result<(), EthApiError>
where
    DB::Error: Into<EthApiError>,
{
    for (address, account_override) in state_overrides {
        apply_account_override(db, address, account_override)?;
    }

    Ok(())
}

fn apply_account_override<DB: Database>(
    db: &mut RevmState<DB>,
    address: Address,
    account_override: AccountOverride,
) -> Result<(), EthApiError>
where
    DB::Error: Into<EthApiError>,
{
    let AccountOverride {
        balance,
        nonce,
        code,
        state,
        state_diff,
        move_precompile_to,
    } = account_override;

    if let Some(move_precompile_to) = move_precompile_to {
        return Err(invalid_override_params(format!(
            "movePrecompileToAddress is not supported: {move_precompile_to}"
        )));
    }

    let mut info = db.basic(address).map_err(Into::into)?.unwrap_or_default();
    if let Some(nonce) = nonce {
        info.nonce = nonce;
    }
    if let Some(code) = code {
        let bytecode = Bytecode::new_raw_checked(code).map_err(|err| {
            invalid_override_params(format!("Invalid account override bytecode: {err}"))
        })?;
        info.set_code(bytecode);
    }
    if let Some(balance) = balance {
        info.balance = balance;
    }

    let mut patched_account = Account {
        info,
        status: AccountStatus::Touched,
        storage: Default::default(),
        transaction_id: 0,
    };

    let storage_overrides = match (state, state_diff) {
        (Some(_), Some(_)) => {
            return Err(invalid_override_params(format!(
                "Both 'state' and 'stateDiff' are set for account {address}"
            )));
        }
        (Some(state), None) => {
            db.commit(RevmHashMap::from_iter([(
                address,
                Account {
                    status: AccountStatus::SelfDestructed | AccountStatus::Touched,
                    ..Default::default()
                },
            )]));
            patched_account.mark_created();
            Some(state)
        }
        (None, Some(state_diff)) => Some(state_diff),
        (None, None) => None,
    };

    if let Some(storage_overrides) = storage_overrides {
        for (slot, value) in storage_overrides {
            patched_account.storage.insert(
                slot.into(),
                EvmStorageSlot::new_changed((!value).into(), value.into(), 0),
            );
        }
    }

    db.commit(RevmHashMap::from_iter([(address, patched_account)]));
    Ok(())
}

pub(crate) fn get_cfg_env_template() -> CfgEnv {
    let mut cfg_env = CfgEnv::default();
    // Reth sets this to true and uses only timeout, but other clients use this as a part of DOS attacks protection, with 100mln gas limit
    // https://github.com/paradigmxyz/reth/blob/62f39a5a151c5f4ddc9bf0851725923989df0412/crates/rpc/rpc/src/eth/revm_utils.rs#L215
    cfg_env.disable_block_gas_limit = false;
    cfg_env.disable_eip3607 = true;
    cfg_env.disable_base_fee = true;
    cfg_env.chain_id = config_value!("CHAIN_ID");
    cfg_env.limit_contract_code_size = None;
    cfg_env.memory_limit = 50 * 1024 * 1024; // 50MB
    cfg_env
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_call_fee_fields_rejects_conflicting_fields() {
        let request = TransactionRequest {
            gas_price: Some(1),
            max_fee_per_gas: Some(1),
            ..Default::default()
        };

        let err = validate_call_fee_fields(&request).unwrap_err();

        assert!(matches!(err, EthApiError::ConflictingFeeFieldsInRequest));
    }

    #[test]
    fn validate_call_fee_fields_rejects_tip_above_fee_cap() {
        let request = TransactionRequest {
            max_fee_per_gas: Some(1),
            max_priority_fee_per_gas: Some(2),
            ..Default::default()
        };

        let err = validate_call_fee_fields(&request).unwrap_err();

        assert!(matches!(
            err,
            EthApiError::InvalidTransaction(RpcInvalidTransactionError::TipAboveFeeCap)
        ));
    }

    #[test]
    fn validate_simulation_max_fee_against_base_fee_rejects_fee_cap_below_base_fee() {
        let request = TransactionRequest {
            max_fee_per_gas: Some(9),
            max_priority_fee_per_gas: Some(0),
            ..Default::default()
        };
        let block_env = BlockEnv {
            basefee: 10,
            ..Default::default()
        };

        let err =
            validate_simulation_max_fee_against_base_fee(&request, &block_env, true).unwrap_err();

        assert!(matches!(
            err,
            EthApiError::InvalidTransaction(RpcInvalidTransactionError::FeeCapTooLow)
        ));
    }

    #[test]
    fn validate_simulation_max_fee_against_base_fee_allows_fee_cap_below_base_fee_when_inactive() {
        let request = TransactionRequest {
            max_fee_per_gas: Some(9),
            max_priority_fee_per_gas: Some(0),
            ..Default::default()
        };
        let block_env = BlockEnv {
            basefee: 10,
            ..Default::default()
        };

        validate_simulation_max_fee_against_base_fee(&request, &block_env, false)
            .expect("inactive fee-cap gate should not reject the request");
    }

    #[test]
    fn validate_simulation_max_fee_against_base_fee_keeps_legacy_gas_price_behavior() {
        let request = TransactionRequest {
            gas_price: Some(9),
            ..Default::default()
        };
        let block_env = BlockEnv {
            basefee: 10,
            ..Default::default()
        };

        validate_simulation_max_fee_against_base_fee(&request, &block_env, true)
            .expect("legacy gasPrice should not be rejected by the base-fee check");
    }

    #[test]
    fn validate_simulation_max_fee_against_base_fee_ignores_requests_without_fee_cap() {
        validate_simulation_max_fee_against_base_fee(
            &TransactionRequest::default(),
            &BlockEnv::default(),
            true,
        )
        .expect("requests without maxFeePerGas should not be rejected");
    }
}
