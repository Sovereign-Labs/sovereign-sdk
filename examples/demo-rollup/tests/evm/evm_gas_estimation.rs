use crate::evm::evm_test_helper::setup;
use crate::evm::evm_test_helper::EVM_EXTENSION;
use ethers::abi::Address;

#[tokio::test(flavor = "multi_thread")]
async fn simple_transfer() -> anyhow::Result<()> {
    let (_, test_client, _) = setup(0, EVM_EXTENSION).await;

    let simple_transfer = test_client.make_tx(Some(Address::zero()), None);
    let gas_estimation = test_client.eth_estimate_gas(simple_transfer).await;
    assert_eq!(gas_estimation, 100_000); // Simple transfer consumes 0 EVM gas so we only see the ABSOLUTE_MARGIN
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn contract_deploy() -> anyhow::Result<()> {
    let (_, test_client, _) = setup(0, EVM_EXTENSION).await;

    let deploy_tx = test_client.make_tx(None, Some(test_client.contract.byte_code()));
    let gas_estimation = test_client.eth_estimate_gas(deploy_tx).await;
    assert_eq!(gas_estimation, (206_448 / 2) * 3 + 100_000);
    Ok(())
}
