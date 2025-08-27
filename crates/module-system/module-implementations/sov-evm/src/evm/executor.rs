use std::convert::Infallible;

use alloy_primitives::Address;
use reth_primitives::TransactionSigned;
#[cfg(feature = "native")]
use revm::context::{result::ResultAndState, TxEnv};
use revm::{
    context::{
        result::{EVMError, ExecutionResult},
        BlockEnv, CfgEnv, Context,
    },
    Database, DatabaseCommit, ExecuteCommitEvm, MainContext,
};

use crate::{evm::conversions::create_tx_env, get_spec_id, sov_evm::SovEvm, EvmRuntimeConfig};

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
    let spec = get_spec_id(cfg.hardforks, block_env.number.to::<u64>());
    cfg_env.with_spec(spec)
}

/// Execute an Ethereum transaction and commit it to the database.
pub fn execute_tx<DB: Database<Error = Infallible> + DatabaseCommit>(
    sov_nonce: u64,
    db: DB,
    block_env: &BlockEnv,
    tx: &TransactionSigned,
    signer: Address,
    cfg: CfgEnv,
) -> Result<ExecutionResult, EVMError<Infallible>> {
    let tx_env = create_tx_env(sov_nonce, tx, signer);
    let context = Context::mainnet()
        .with_db(db)
        .with_block(block_env)
        .with_cfg(cfg);
    let mut evm = SovEvm::new(context, ());

    evm.transact_commit(tx_env)
}

#[cfg(feature = "native")]
pub(crate) fn inspect<DB: Database<Error = Infallible> + DatabaseCommit>(
    db: DB,
    block_env: &BlockEnv,
    tx: TxEnv,
    cfg: CfgEnv,
) -> Result<ResultAndState, EVMError<Infallible>> {
    use revm::InspectEvm;

    let config = revm_inspectors::tracing::TracingInspectorConfig::all();
    let inspector = revm_inspectors::tracing::TracingInspector::new(config);

    let context = Context::mainnet()
        .with_db(db)
        .with_block(block_env)
        .with_cfg(cfg);
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
        expected_cfg_env.limit_contract_code_size = Some(100);
        expected_cfg_env.spec = SpecId::SHANGHAI;

        assert_eq!(expected_cfg_env, cfg_env);
    }
}
