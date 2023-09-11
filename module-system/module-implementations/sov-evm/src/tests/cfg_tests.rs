use revm::primitives::{CfgEnv, SpecId, U256};

use crate::call::{get_cfg_env, get_spec_id};
use crate::evm::transaction::BlockEnv;
use crate::evm::EvmChainConfig;

#[test]
fn cfg_test() {
    let block_env = BlockEnv {
        number: 10,
        ..Default::default()
    };

    let cfg = EvmChainConfig {
        limit_contract_code_size: Some(100),
        spec: vec![(0, SpecId::SHANGHAI)].into_iter().collect(),
        ..Default::default()
    };

    let template_cfg = CfgEnv {
        chain_id: U256::from(2),
        disable_base_fee: true,
        ..Default::default()
    };

    let cfg_env = get_cfg_env(&block_env, cfg, Some(template_cfg));

    assert_eq!(
        cfg_env,
        CfgEnv {
            chain_id: U256::from(1),
            disable_base_fee: true,
            spec_id: SpecId::SHANGHAI,
            limit_contract_code_size: Some(100),
            ..Default::default()
        }
    );
}

#[test]
fn spec_id_lookup() {
    let spec = vec![
        (0, SpecId::CONSTANTINOPLE),
        (10, SpecId::BERLIN),
        (20, SpecId::LONDON),
    ];

    assert_eq!(get_spec_id(spec.clone(), 0), SpecId::CONSTANTINOPLE);
    assert_eq!(get_spec_id(spec.clone(), 5), SpecId::CONSTANTINOPLE);
    assert_eq!(get_spec_id(spec.clone(), 10), SpecId::BERLIN);
    assert_eq!(get_spec_id(spec.clone(), 15), SpecId::BERLIN);
    assert_eq!(get_spec_id(spec.clone(), 20), SpecId::LONDON);
    assert_eq!(get_spec_id(spec, 25), SpecId::LONDON);
}
