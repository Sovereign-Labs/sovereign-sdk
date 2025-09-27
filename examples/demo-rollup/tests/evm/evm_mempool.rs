use crate::evm::evm_test_helper::setup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use ethers::abi::Address;

#[tokio::test(flavor = "multi_thread")]
async fn out_of_order_nonces() -> anyhow::Result<()> {
    let (_, test_client, _) = setup(0, EVM_EXTENSION).await;

    let mut tx = test_client.make_tx(Some(Address::zero()), None);
    let tx_0 = tx.set_nonce(0).clone();
    let tx_1 = tx.set_nonce(1).clone();

    test_client.send_tx(tx_1).await.unwrap();
    test_client.send_tx(tx_0).await.unwrap();
    Ok(())
}
