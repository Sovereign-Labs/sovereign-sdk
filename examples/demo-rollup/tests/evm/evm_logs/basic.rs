use alloy_provider::Provider;
use alloy_rpc_types_eth::{Filter, Log};

use super::{emit, setup};

#[tokio::test(flavor = "multi_thread")]
async fn single_log() -> anyhow::Result<()> {
    let (client, contract, _) = setup().await?;

    let receipt = emit(&contract, 1, 0).await?;
    let [log]: [Log; 1] = client.get_logs(&Filter::new()).await?.try_into().unwrap();

    assert_eq!(log.log_index, Some(0));
    assert!(!log.removed);
    assert_eq!(log.transaction_index, Some(1));
    assert_eq!(log.transaction_hash, Some(receipt.transaction_hash));
    assert_eq!(log.block_number, receipt.block_number);
    assert_eq!(log.block_hash, None);
    assert!(log.block_timestamp.unwrap() > 0);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn two_logs() -> anyhow::Result<()> {
    let (client, contract, _) = setup().await?;

    let _ = emit(&contract, 2, 0).await?;
    let [log0, log1]: [Log; 2] = client.get_logs(&Filter::new()).await?.try_into().unwrap();

    assert_eq!(log0.log_index, Some(0));
    assert_eq!(log1.log_index, Some(1));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn two_transactions() -> anyhow::Result<()> {
    let (client, contract, _) = setup().await?;

    let _ = emit(&contract, 1, 0).await?;
    let _ = emit(&contract, 1, 0).await?;
    let [log0, log1]: [Log; 2] = client.get_logs(&Filter::new()).await?.try_into().unwrap();

    assert_eq!(log0.log_index, Some(0));
    assert_eq!(log0.transaction_index, Some(1));
    assert_eq!(log1.log_index, Some(1)); // Log index is block-wide
    assert_eq!(log1.transaction_index, Some(2));

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn two_transactions_in_two_blocks() -> anyhow::Result<()> {
    let (client, contract, rollup) = setup().await?;

    let _ = emit(&contract, 1, 0).await?;
    rollup.wait_for_next_blocks(1).await;
    let _ = emit(&contract, 1, 0).await?;

    let all_blocks = Filter::new().select(0..);
    let [log0, log1]: [Log; 2] = client.get_logs(&all_blocks).await?.try_into().unwrap();

    assert_eq!(log0.log_index, Some(0));
    assert_eq!(log0.transaction_index, Some(1));
    assert_eq!(log1.log_index, Some(0)); // Log index is block-wide
    assert_eq!(log1.transaction_index, Some(0));

    Ok(())
}
