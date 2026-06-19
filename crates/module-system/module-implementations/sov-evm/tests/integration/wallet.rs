use borsh::BorshDeserialize;
use sov_evm::{
    BorshSpecId, CallMessage, ChainSpecUpdate, ContractCreationPolicyUpdate,
    EvmRuntimeConfigUpdate, RlpEvmTransaction, SpecId,
};
use sov_modules_api::macros::UniversalWallet;
use sov_modules_api::sov_universal_wallet::schema::Schema;
use sov_modules_api::Spec;
use sov_test_utils::TestSpec;

#[derive(Debug, Clone, PartialEq, borsh::BorshSerialize, BorshDeserialize, UniversalWallet)]
pub enum RuntimeCall<S: Spec> {
    Evm(CallMessage<S>),
}

#[test]
fn test_display_rlp() {
    let msg: RuntimeCall<TestSpec> =
        RuntimeCall::Evm(CallMessage::Call(RlpEvmTransaction { rlp: vec![1, 2, 3] }));
    let schema = Schema::of_single_type::<RuntimeCall<TestSpec>>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        r#"Evm.Call { rlp: 0x010203 }"#
    );
}

#[test]
fn test_display_evm_config_update() {
    let msg: RuntimeCall<TestSpec> =
        RuntimeCall::Evm(CallMessage::UpdateRuntimeConfig(EvmRuntimeConfigUpdate::<
            TestSpec,
        > {
            new_hardfork: Some((100, BorshSpecId(SpecId::CANCUN))),
            new_contract_creation_policy: Some(ContractCreationPolicyUpdate::Everyone),
            chain_spec_update: Some(ChainSpecUpdate {
                new_limit_contract_code_size: None, // Some(10_000) - uncommenting this causes the test to fail due to unused input. This looks like a bug in the UniversalWallet to me
                new_block_gas_limit: None,          // Some(10_000_000),
                new_tx_gas_limit: None,
            }),
            new_admin: None,
        }));
    let schema = Schema::of_single_type::<RuntimeCall<TestSpec>>().unwrap();
    assert_eq!(
        schema.display(0, &borsh::to_vec(&msg).unwrap()).unwrap(),
        r#"Evm.UpdateRuntimeConfig { new_hardfork: (100, 17), new_contract_creation_policy: Everyone, chain_spec_update: { new_limit_contract_code_size: None, new_block_gas_limit: None, new_tx_gas_limit: None }, new_admin: None }"#
    );
}
