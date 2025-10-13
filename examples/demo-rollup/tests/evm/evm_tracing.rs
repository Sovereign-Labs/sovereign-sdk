use alloy_primitives::utils::parse_ether;
use alloy_primitives::Address;
use alloy_provider::ext::DebugApi;
use alloy_rpc_types_trace::geth::{CallConfig, GethDebugTracingOptions};
use sov_test_utils::Erc20;

use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;

#[tokio::test(flavor = "multi_thread")]
async fn debug_trace_block_by_number() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let client = alloy_client(rollup.http_addr);
    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    let mint_tx = usdc.mint(Address::ZERO, parse_ether("1")?).send().await?;
    rollup.wait_for_next_blocks(1).await; // Block nr 2 mined with 2 transactions
    rollup.pause_preferred_batches().await;

    let opts = GethDebugTracingOptions::call_tracer(CallConfig::default());
    let trace = client
        .debug_trace_block_by_number(1.into(), opts.clone())
        .await?;
    assert_eq!(trace.len(), 0);

    let trace = client.debug_trace_block_by_number(2.into(), opts).await?;
    assert_eq!(trace.len(), 2);
    assert_eq!(trace[1].tx_hash().unwrap(), *mint_tx.tx_hash());

    Ok(())
}
