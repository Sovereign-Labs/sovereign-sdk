use crate::{
    get_spec_id,
    sov_evm::{PrecompileDb, SovEvm, SovPrecompiles, StorageAccessInspector},
    EvmRuntimeConfig,
};
use revm::InspectEvm;
use revm::{context::TxEnv, inspector::InspectorEvmTr};
use revm::{
    context::{
        result::{EVMError, ExecResultAndState, ExecutionResult},
        BlockEnv, CfgEnv, Context,
    },
    Database, MainContext,
};
#[cfg(feature = "native")]
use revm::{interpreter::interpreter::EthInterpreter, Inspector};
use revm_database_interface::{DBErrorMarker, TryDatabaseCommit};
use sov_address::{EthereumAddress, FromVmAddress};
use sov_modules_api::macros::config_value;
use sov_modules_api::Spec;

/// The maximum contract code size is 512KiB by default.
pub const DEFAULT_MAX_CONTRACT_CODE_SIZE: usize = 512 * 1024;

/// builds CfgEnv
/// Returns correct config depending on spec for given block number
// Copies context-dependent values from template_cfg or default if not provided
pub(crate) fn get_cfg_env(
    block_env: &BlockEnv,
    cfg: &EvmRuntimeConfig,
    template_cfg: Option<CfgEnv>,
) -> CfgEnv {
    let mut cfg_env = template_cfg.unwrap_or_default();
    cfg_env.chain_id = config_value!("CHAIN_ID");
    cfg_env.memory_limit = 50 * 1024 * 1024; // 50MB
    cfg_env.limit_contract_code_size = Some(
        cfg.chain_spec
            .limit_contract_code_size
            .unwrap_or(DEFAULT_MAX_CONTRACT_CODE_SIZE),
    );
    cfg_env.tx_chain_id_check = false;
    let spec = get_spec_id(&cfg.hardforks, block_env.number.to::<u64>());
    cfg_env.with_spec(spec)
}

/// Execute an Ethereum transaction with sovereign precompiles and commit it to the database.
pub fn transact_commit<'a, S, DB, E>(
    mut db: &mut DB,
    block_env: &'a BlockEnv,
    tx: TxEnv,
    cfg: CfgEnv,
    precompiles: SovPrecompiles<'a, S>,
) -> Result<ExecutionResult, EVMError<E>>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
    DB: Database<Error = E> + TryDatabaseCommit<Error = E> + PrecompileDb<S>,
    E: DBErrorMarker,
{
    let ExecResultAndState { result, state } =
        transact_with_precompiles(&mut db, block_env, tx, cfg, precompiles)?;
    // We don't use revm's transact_commit as it does not support returning an error
    db.try_commit(state)?;
    Ok(result)
}

/// Execute ethereum transaction with inspection and sovereign precompiles.
///
/// This variant uses `SovPrecompiles` which can handle stateful precompiles
/// that access sovereign SDK state during EVM execution, combined with
/// a custom inspector for debugging/tracing.
#[cfg(feature = "native")]
pub(crate) fn inspect_with_precompiles<'a, S, DB, E, I>(
    db: DB,
    block_env: &'a BlockEnv,
    tx: TxEnv,
    cfg: CfgEnv,
    inspector: I,
    precompiles: SovPrecompiles<'a, S>,
) -> Result<ExecResultAndState<ExecutionResult>, EVMError<E>>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
    DB: Database<Error = E> + PrecompileDb<S>,
    E: DBErrorMarker,
    I: Inspector<Context<&'a BlockEnv, TxEnv, CfgEnv, DB>, EthInterpreter>,
{
    let context = context(db, block_env, cfg);
    let storage_inspector = StorageAccessInspector::new();
    let mut evm = SovEvm::with_precompiles(context, (inspector, storage_inspector), precompiles);
    let mut exec_result = evm.inspect_tx(tx)?;
    // Rebate the gas we charged for storage access during execution. We rebate after rather than during execution so that
    // a loop of SSTORE/SLOADs will still terminate due to OOG despite the rebate.
    rebate_gas(
        &mut exec_result,
        evm.inspector().1.gas_spent_on_storage_access(),
    );
    Ok(exec_result)
}

/// Execute ethereum transaction with sovereign precompiles.
///
/// This variant uses `SovPrecompiles` which can handle stateful precompiles
/// that access sovereign SDK state during EVM execution.
pub fn transact_with_precompiles<'a, S, DB, E>(
    db: DB,
    block_env: &'a BlockEnv,
    tx: TxEnv,
    cfg: CfgEnv,
    precompiles: SovPrecompiles<'a, S>,
) -> Result<ExecResultAndState<ExecutionResult>, EVMError<E>>
where
    S: Spec,
    S::Address: FromVmAddress<EthereumAddress>,
    DB: Database<Error = E> + PrecompileDb<S>,
    E: DBErrorMarker,
{
    let context = context(db, block_env, cfg);
    let mut evm = SovEvm::with_precompiles(context, StorageAccessInspector::new(), precompiles);
    let mut exec_result = evm.inspect_tx(tx)?;
    // Rebate the gas we charged for storage access during execution. We rebate after rather than during execution so that
    // a loop of SSTORE/SLOADs will still terminate due to OOG despite the rebate.
    rebate_gas(
        &mut exec_result,
        evm.inspector().gas_spent_on_storage_access(),
    );
    Ok(exec_result)
}

fn context<DB: Database<Error = E>, E: DBErrorMarker>(
    db: DB,
    block_env: &BlockEnv,
    cfg: CfgEnv,
) -> Context<&BlockEnv, TxEnv, CfgEnv, DB> {
    Context::mainnet()
        .with_db(db)
        .with_block(block_env)
        .with_cfg(cfg)
}

fn rebate_gas(exec_result: &mut ExecResultAndState<ExecutionResult>, gas_to_rebate: u64) {
    match &mut exec_result.result {
        ExecutionResult::Success { gas_used, .. } => {
            *gas_used = gas_used.saturating_sub(gas_to_rebate);
        }
        ExecutionResult::Revert { gas_used, .. } => {
            *gas_used = gas_used.saturating_sub(gas_to_rebate);
        }
        ExecutionResult::Halt { gas_used, .. } => {
            *gas_used = gas_used.saturating_sub(gas_to_rebate);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use revm::primitives::hardfork::SpecId;
    use sov_modules_api::macros::config_value;

    use crate::ContractCreationPolicy;

    use super::*;

    #[test]
    fn cfg_test() {
        let block_env = BlockEnv {
            number: U256::from(10),
            ..Default::default()
        };

        let cfg = EvmRuntimeConfig {
            chain_spec: crate::EvmChainSpec {
                limit_contract_code_size: Some(100),
                ..Default::default()
            },
            hardforks: vec![(0, SpecId::CANCUN)],
            contract_creation_policy: ContractCreationPolicy::Everyone,
        };

        let mut template_cfg_env = CfgEnv::default();
        template_cfg_env.chain_id = 2;
        template_cfg_env.disable_base_fee = true;

        let cfg_env = get_cfg_env(&block_env, &cfg, Some(template_cfg_env));

        let mut expected_cfg_env = CfgEnv::default();
        expected_cfg_env.chain_id = config_value!("CHAIN_ID");
        expected_cfg_env.tx_chain_id_check = false;
        expected_cfg_env.disable_base_fee = true;
        expected_cfg_env.limit_contract_code_size = Some(100);
        expected_cfg_env.spec = SpecId::CANCUN;
        expected_cfg_env.memory_limit = 50 * 1024 * 1024; // 50MB

        assert_eq!(expected_cfg_env, cfg_env);
    }
}
