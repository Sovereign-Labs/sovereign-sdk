use alloy_consensus::{TxEip1559, TypedTransaction};
use alloy_eips::eip1559::MIN_PROTOCOL_BASE_FEE;
use alloy_primitives::{Bytes, TxKind};
use sov_evm::{EthereumAuthenticator, Evm};
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::RawTx;
use sov_test_utils::{SimpleStorage, TransactionTestCase, TransactionType};

use crate::helpers::setup;
use crate::runtime::{RT, S};

#[test]
fn test_block_updates() {
    let (mut runner, account, _) = setup();
    let contract = SimpleStorage::default();
    let create_contract_tx_request = TypedTransaction::Eip1559(TxEip1559 {
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
    let (signed_eth_tx, _) = account.sign(create_contract_tx_request);
    let create_contract_tx = RawTx {
        data: borsh::to_vec(&signed_eth_tx).unwrap(),
    };

    runner.execute_transaction(TransactionTestCase {
        input: TransactionType::<RT, S>::PreAuthenticated(RT::encode_with_ethereum_auth(
            create_contract_tx,
        )),
        assert: Box::new(move |_result, state| {
            let evm = Evm::<S>::default();
            assert!(evm.pending_head(state).is_none());
            let block_0 = evm.blocks.get(&0, state).unwrap_infallible().unwrap();
            let block_1 = evm.blocks.get(&1, state).unwrap_infallible().unwrap();
            let prev_block = &block_0;
            let current_block = &block_1;
            assert!(prev_block != current_block,);
            assert_eq!(
                current_block.header().parent_hash,
                prev_block.header().hash()
            );
            assert!(!current_block.header().transaction_root_is_empty(),);
            assert_eq!(current_block.header().number, 1);
            let txs = current_block.transactions();
            assert_eq!(txs.start, 0);
            assert_eq!(txs.end, 1);
            let block_height = evm.get_block_height_by_hash(&current_block.header().hash(), state);
            assert_eq!(block_height, Some(1));
        }),
    });
}
