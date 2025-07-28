use reth_primitives::{TxKind, U256};
use reth_rpc_types::transaction::EIP1559TransactionRequest;
use reth_rpc_types::TypedTransactionRequest;
use revm::primitives::{BlockEnv, CfgEnv, CfgEnvWithHandlerCfg, ExecutionResult};
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
    let tx_request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
        chain_id: config_value!("CHAIN_ID"),
        nonce: 0,
        max_priority_fee_per_gas: Default::default(),
        max_fee_per_gas: U256::from(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2),
        gas_limit: U256::from(1_000_000u64),
        kind: TxKind::Create,
        value: Default::default(),
        input: reth_primitives::Bytes::from(contract.byte_code().to_vec()),
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
        let tx_request = TypedTransactionRequest::EIP1559(EIP1559TransactionRequest {
            chain_id: config_value!("CHAIN_ID"),
            nonce: 1,
            max_priority_fee_per_gas: Default::default(),
            max_fee_per_gas: U256::from(reth_primitives::constants::MIN_PROTOCOL_BASE_FEE * 2),
            gas_limit: U256::from(1_000_000u64),
            kind: TxKind::Call(contract_addr),
            value: Default::default(),
            input: reth_primitives::Bytes::from(
                hex::decode(hex::encode(contract.failing_function_call_data())).unwrap(),
            ),
            access_list: Default::default(),
        });
        let (signed_eth_tx, _) = account.sign(tx_request);
        let mut cfg_env_with_handler = CfgEnvWithHandlerCfg::new(
            CfgEnv::default(),
            reth_primitives::revm_primitives::HandlerCfg {
                spec_id: SpecId::SHANGHAI,
            },
        );
        cfg_env_with_handler.chain_id = config_value!("CHAIN_ID");
        let result = executor::execute_tx(
            &mut evm_db,
            &BlockEnv::default(),
            &convert_to_transaction_signed(signed_eth_tx).unwrap(),
            account.address(),
            cfg_env_with_handler,
        )
        .unwrap();
        assert!(matches!(result, ExecutionResult::Revert { .. }));
    });
}
