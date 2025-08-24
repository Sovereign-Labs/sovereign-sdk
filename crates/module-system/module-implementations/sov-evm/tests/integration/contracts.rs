use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::{Bytes, TxKind};
use revm::context::result::ExecutionResult;
use revm::context::{BlockEnv, CfgEnv};
use sov_evm::{convert_to_transaction_signed, executor, EthereumAuthenticator, Evm, SpecId};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::{SimpleStorageContract, TransactionType};

use crate::helpers::setup;
use crate::runtime::{RT, S};

#[test]
fn test_invalid_contract_execution() {
    let (mut runner, _, account, _) = setup();
    let contract = SimpleStorageContract::default();
    let contract_addr = account.address().create(0);
    let tx_request = TypedTransaction::Eip1559(TxEip1559 {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 0,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        gas_limit: 1_000_000,
        to: TxKind::Create,
        value: Default::default(),
        input: Bytes::from(contract.byte_code().to_vec()),
        access_list: Default::default(),
    });
    let (signed_eth_tx, _) = account.sign(tx_request);
    let raw_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    };

    runner.execute(TransactionType::<RT, S>::PreAuthenticated(
        RT::encode_with_ethereum_auth(raw_tx),
    ));

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let mut evm_db = evm.get_db(state);
        let tx_request = TypedTransaction::Eip1559(TxEip1559 {
            chain_id: config_value!("CHAIN_ID"),
            nonce: 1,
            max_priority_fee_per_gas: Default::default(),
            max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
            gas_limit: 1_000_000,
            to: TxKind::Call(contract_addr),
            value: Default::default(),
            input: Bytes::from(
                hex::decode(hex::encode(contract.failing_function_call_data())).unwrap(),
            ),
            access_list: Default::default(),
        });
        let (signed_eth_tx, _) = account.sign(tx_request);
        let cfg_env =
            CfgEnv::new_with_spec(SpecId::SHANGHAI).with_chain_id(config_value!("CHAIN_ID"));
        let result = executor::execute_tx(
            1,
            &mut evm_db,
            &BlockEnv::default(),
            &convert_to_transaction_signed(signed_eth_tx).unwrap(),
            account.address(),
            cfg_env,
        )
        .unwrap();
        assert!(matches!(result, ExecutionResult::Revert { .. }));
    });
}
