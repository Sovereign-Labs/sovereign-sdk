use reth_primitives::revm_primitives::{BlockEnv, CfgEnv, HandlerCfg, SpecId, U256};
use sov_modules_api::macros::config_value;

use crate::call::get_cfg_env_with_handler;
use crate::evm::EvmChainConfig;
use crate::get_spec_id;

#[test]
fn cfg_test() {
    let block_env = BlockEnv {
        number: U256::from(10),
        ..Default::default()
    };

    let cfg = EvmChainConfig {
        limit_contract_code_size: Some(100),
        spec: vec![(0, SpecId::SHANGHAI)].into_iter().collect(),
        ..Default::default()
    };

    let mut template_cfg_env = CfgEnv::default();
    template_cfg_env.chain_id = 2;
    template_cfg_env.disable_base_fee = true;

    let cfg_env_with_hanlder = get_cfg_env_with_handler(&block_env, cfg, Some(template_cfg_env));

    let mut expected_cfg_env = CfgEnv::default();
    expected_cfg_env.chain_id = config_value!("CHAIN_ID");
    expected_cfg_env.disable_base_fee = true;
    expected_cfg_env.limit_contract_code_size = Some(100);

    assert_eq!(expected_cfg_env, cfg_env_with_hanlder.cfg_env);

    let expected_handler_cfg = HandlerCfg {
        spec_id: SpecId::SHANGHAI,
    };

    assert_eq!(expected_handler_cfg, cfg_env_with_hanlder.handler_cfg);
}

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
