use crate::helpers::{setup, EvmAccount};
use crate::runtime::{RT, S};
use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_eips::BlockId;
use alloy_primitives::{Bytes, TxKind, B256, U256};
use alloy_rpc_types::TransactionRequest;
use sov_evm::{EthereumAuthenticator, Evm};
use sov_modules_api::macros::config_value;
use sov_modules_api::RawTx;
use sov_test_utils::TransactionType;

const BASEFEE_CONTRACT_INIT_CODE_HEX: &str = "6a60004860005260206000f3600052600b6015f3";

struct TxWithHash {
    tx: TransactionType<RT, S>,
    hash: B256,
}

fn create_deploy_tx_with_init_code(
    nonce: u64,
    account: &EvmAccount,
    init_code: Bytes,
) -> TxWithHash {
    let tx = TxEip1559 {
        input: init_code,
        nonce,
        gas_limit: 1_000_000,
        max_fee_per_gas: MIN_PROTOCOL_BASE_FEE as u128 * 2,
        chain_id: config_value!("CHAIN_ID"),
        ..Default::default()
    };
    let (signed_eth_tx, signed_tx) = account.sign(TypedTransaction::Eip1559(tx));
    let raw_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).expect("RLP tx should serialize"),
    };
    TxWithHash {
        tx: TransactionType::PreAuthenticated(RT::encode_with_ethereum_auth(raw_tx)),
        hash: *signed_tx.hash(),
    }
}

#[test]
fn test_eth_call_basefee_opcode_matches_block_header_base_fee() {
    let (mut runner, account, _, _) = setup();

    // init code returns this runtime:
    //   PUSH1 0x00
    //   BASEFEE
    //   PUSH1 0x00
    //   MSTORE
    //   PUSH1 0x20
    //   PUSH1 0x00
    //   RETURN
    let init_code = Bytes::from(
        hex::decode(BASEFEE_CONTRACT_INIT_CODE_HEX).expect("BASEFEE init code should be valid"),
    );
    let deploy_tx = create_deploy_tx_with_init_code(0, &account, init_code);
    let deploy_tx_hash = deploy_tx.hash;
    runner.execute(deploy_tx.tx);

    let caller = account.address();

    runner.query_visible_state(|state| {
        let evm = Evm::<S>::default();
        let receipt = evm
            .get_transaction_receipt(deploy_tx_hash, state)
            .unwrap()
            .expect("Deployment tx should exist");
        let contract_address = receipt
            .contract_address
            .expect("BASEFEE test contract deployment transaction failed");

        let deployed_code = evm
            .get_code(contract_address, Some(BlockId::number(1)), state)
            .unwrap();
        assert!(
            !deployed_code.is_empty(),
            "BASEFEE test contract was not deployed"
        );

        let block = evm
            .get_block_by_number(Some(BlockId::number(1)), Some(false), state)
            .unwrap()
            .expect("Block 1 should exist after executing one transaction");
        let expected_base_fee = block
            .header
            .base_fee_per_gas
            .expect("Sealed block should expose base fee");
        assert!(
            expected_base_fee > 0,
            "Test precondition failed: block base fee is zero"
        );

        let output = evm
            .eth_call(
                TransactionRequest {
                    from: Some(caller),
                    to: Some(TxKind::Call(contract_address)),
                    ..Default::default()
                },
                Some(BlockId::number(1)),
                None,
                None,
                state,
            )
            .unwrap();
        let observed_base_fee = U256::from_be_slice(output.as_ref());

        assert_eq!(
            observed_base_fee,
            U256::from(expected_base_fee),
            "eth_call BASEFEE opcode output does not match the block header base fee"
        );
    });
}
