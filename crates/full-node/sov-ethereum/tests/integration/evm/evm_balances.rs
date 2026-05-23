use std::str::FromStr;

use alloy_primitives::{Address, U256};
use sov_address::{EthereumAddress, MultiAddressEvm};
use sov_bank::config_gas_token_id;
use sov_test_utils::test_rollup::TestRollup;

use crate::common::{setup_with_simple_storage, EVM_EXTENSION};
use crate::runtime::EvmBlueprint;

const RECEIVER_ADDR_STR: &str = "0x3FE0233e6cf3c9753fcB7449987EC49C88aDDE71";

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_balances() -> anyhow::Result<()> {
    let (test_rollup, evm_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;
    let sender_address = evm_client.address();

    let receiver_address = Address::from_str(RECEIVER_ADDR_STR).unwrap();

    let (sender_bank_balance_start, sender_evm_balance_start) =
        get_balances(sender_address, &test_rollup, &evm_client).await;

    let (receiver_bank_balance_start, receiver_evm_balance_start) =
        get_balances(receiver_address, &test_rollup, &evm_client).await;

    let eth_to_send = 2u128;
    evm_client
        .send_eth(receiver_address, U256::from(eth_to_send))
        .await;
    test_rollup.wait_for_rollup_height_advance_by(2).await;

    let (sender_bank_balance_end, sender_evm_balance_end) =
        get_balances(sender_address, &test_rollup, &evm_client).await;

    let (receiver_bank_balance_end, receiver_evm_balance_end) =
        get_balances(receiver_address, &test_rollup, &evm_client).await;

    assert_eq!(sender_bank_balance_start, sender_evm_balance_start);
    assert_eq!(sender_bank_balance_end, sender_evm_balance_end);
    assert!(sender_bank_balance_start > sender_bank_balance_end + eth_to_send);

    assert_eq!(receiver_bank_balance_start, receiver_evm_balance_start);
    assert_eq!(
        receiver_bank_balance_start + eth_to_send,
        receiver_bank_balance_end
    );
    assert_eq!(
        receiver_evm_balance_start + eth_to_send,
        receiver_evm_balance_end
    );

    Ok(())
}

async fn get_balances(
    address: Address,
    test_rollup: &TestRollup<EvmBlueprint>,
    evm_client: &sov_eth_client::SimpleStorageClient,
) -> (u128, u128) {
    let sov_to_addr = MultiAddressEvm::Vm(EthereumAddress::new(address.0 .0));

    let token_id = config_gas_token_id();

    let bank_balance = test_rollup
        .client
        .get_balance::<crate::runtime::EvmTestSpec>(&sov_to_addr, &token_id, None)
        .await
        .unwrap()
        .0;

    let evm_balance: u128 = evm_client
        .eth_get_balance(address)
        .await
        .try_into()
        .unwrap();
    (bank_balance, evm_balance)
}
