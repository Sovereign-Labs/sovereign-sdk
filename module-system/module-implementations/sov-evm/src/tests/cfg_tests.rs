use revm::primitives::{CfgEnv, SpecId, U256};

use crate::call::{get_cfg_env, get_spec_id};
use crate::evm::transaction::BlockEnv;
use crate::evm::EvmChainCfg;
use crate::SpecIdWrapper;

#[test]
fn cfg_test() {
    let block_env = BlockEnv {
        number: 10,
        ..Default::default()
    };

    let cfg = EvmChainCfg {
        chain_id: 1,
        limit_contract_code_size: Some(100),
        spec: vec![(0, SpecIdWrapper::new(SpecId::SHANGHAI))]
            .into_iter()
            .collect(),
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
        (0, SpecIdWrapper::new(SpecId::CONSTANTINOPLE)),
        (10, SpecIdWrapper::new(SpecId::BERLIN)),
        (20, SpecIdWrapper::new(SpecId::LONDON)),
    ];

    assert_eq!(
        get_spec_id(spec.clone(), 0),
        SpecIdWrapper::new(SpecId::CONSTANTINOPLE)
    );
    assert_eq!(
        get_spec_id(spec.clone(), 5),
        SpecIdWrapper::new(SpecId::CONSTANTINOPLE)
    );
    assert_eq!(
        get_spec_id(spec.clone(), 10),
        SpecIdWrapper::new(SpecId::BERLIN)
    );
    assert_eq!(
        get_spec_id(spec.clone(), 15),
        SpecIdWrapper::new(SpecId::BERLIN)
    );
    assert_eq!(
        get_spec_id(spec.clone(), 20),
        SpecIdWrapper::new(SpecId::LONDON)
    );
    assert_eq!(get_spec_id(spec, 25), SpecIdWrapper::new(SpecId::LONDON));
}
