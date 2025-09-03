use reth_revm::db::DBErrorMarker;
use revm::context::TxEnv;
use revm::InspectEvm;
use revm::{
    context::{
        result::{EVMError, ExecResultAndState, ExecutionResult},
        BlockEnv, CfgEnv, Context,
    },
    Database, MainContext,
};

use crate::{
    db::commit::FallibleDatabaseCommit,
    get_spec_id,
    sov_evm::{SovEvm, UnmeteredStorageAccessInspector},
    EvmRuntimeConfig,
};

/// builds CfgEnv
/// Returns correct config depending on spec for given block number
// Copies context-dependent values from template_cfg or default if not provided
pub(crate) fn get_cfg_env(
    block_env: &BlockEnv,
    cfg: EvmRuntimeConfig,
    template_cfg: Option<CfgEnv>,
) -> CfgEnv {
    let mut cfg_env = template_cfg.unwrap_or_default();
    cfg_env.chain_id = cfg.chain_spec.chain_id;
    cfg_env.limit_contract_code_size = cfg.chain_spec.limit_contract_code_size;
    cfg_env.disable_block_gas_limit = true;
    cfg_env.disable_balance_check = true;
    let spec = get_spec_id(&cfg.hardforks, block_env.number.to::<u64>());
    cfg_env.with_spec(spec)
}

/// Execute an Ethereum transaction and commit it to the database.
pub fn transact_commit<
    DB: Database<Error = E> + FallibleDatabaseCommit<Error = E>,
    E: DBErrorMarker,
>(
    mut db: DB,
    block_env: &BlockEnv,
    tx: TxEnv,
    cfg: CfgEnv,
) -> Result<ExecutionResult, EVMError<E>> {
    let ExecResultAndState { result, state } = transact(&mut db, block_env, tx, cfg)?;
    // We don't use transact_commit as it does not support returning an error
    db.commit(state)?;
    Ok(result)
}

#[cfg(feature = "native")]
pub(crate) fn call<DB: Database<Error = E>, E: DBErrorMarker>(
    mut db: DB,
    block_env: &BlockEnv,
    tx: TxEnv,
    cfg: CfgEnv,
) -> Result<ExecutionResult, EVMError<E>> {
    Ok(transact(&mut db, block_env, tx, cfg)?.result)
}

fn transact<DB: Database<Error = E>, E: DBErrorMarker>(
    db: &mut DB,
    block_env: &BlockEnv,
    tx: TxEnv,
    cfg: CfgEnv,
) -> Result<ExecResultAndState<ExecutionResult>, EVMError<E>> {
    let context = Context::mainnet()
        .with_db(db)
        .with_block(block_env)
        .with_cfg(cfg);
    let inspector = UnmeteredStorageAccessInspector::new();
    let mut evm = SovEvm::new(context, inspector);
    evm.inspect_tx(tx)
}

#[cfg(test)]
mod tests {
    use alloy_primitives::U256;
    use revm::primitives::hardfork::SpecId;
    use sov_modules_api::macros::config_value;

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
            hardforks: vec![(0, SpecId::SHANGHAI)],
        };

        let mut template_cfg_env = CfgEnv::default();
        template_cfg_env.chain_id = 2;
        template_cfg_env.disable_base_fee = true;

        let cfg_env = get_cfg_env(&block_env, cfg, Some(template_cfg_env));

        let mut expected_cfg_env = CfgEnv::default();
        expected_cfg_env.chain_id = config_value!("CHAIN_ID");
        expected_cfg_env.disable_base_fee = true;
        expected_cfg_env.disable_balance_check = true;
        expected_cfg_env.disable_block_gas_limit = true;
        expected_cfg_env.limit_contract_code_size = Some(100);
        expected_cfg_env.spec = SpecId::SHANGHAI;

        assert_eq!(expected_cfg_env, cfg_env);
    }
}
