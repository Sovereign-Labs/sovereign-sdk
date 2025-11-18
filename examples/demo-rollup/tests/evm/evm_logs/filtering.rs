// TODO:
// * Pending
// * Latest
// * By Address
// * By Single topic
// * By sum of two topics
use alloy_provider::Provider;
use alloy_rpc_types_eth::Filter;

use super::{emit, setup};

#[tokio::test(flavor = "multi_thread")]
async fn by_range() -> anyhow::Result<()> {
    let (client, contract, rollup) = setup().await?;

    let receipt0 = emit(&contract, 1, 0).await?;
    rollup.wait_for_next_blocks(1).await;
    let receipt1 = emit(&contract, 1, 0).await?;

    let only_first_block = Filter::new().select(receipt0.block_number.unwrap());
    let only_last_block = Filter::new().select(receipt1.block_number.unwrap());
    let all_blocks = Filter::new().select(0..);

    assert_eq!(client.get_logs(&only_first_block).await?.len(), 1);
    assert_eq!(client.get_logs(&only_last_block).await?.len(), 1);
    assert_eq!(client.get_logs(&all_blocks).await?.len(), 2);

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn by_hash() -> anyhow::Result<()> {
    let (client, contract, rollup) = setup().await?;

    let receipt0 = emit(&contract, 1, 0).await?;
    rollup.wait_for_next_blocks(1).await;
    let receipt1 = emit(&contract, 1, 0).await?;

    let only_first_block = Filter::new().at_block_hash(receipt0.block_hash.unwrap());
    let only_last_block = Filter::new().at_block_hash(receipt1.block_hash.unwrap());

    assert_eq!(client.get_logs(&only_first_block).await?.len(), 1);
    assert_eq!(client.get_logs(&only_last_block).await?.len(), 1);

    Ok(())
}
