use alloy_primitives::utils::parse_ether;
use alloy_primitives::Address;
use alloy_provider::ext::DebugApi;
use alloy_rpc_types_eth::BlockNumberOrTag;
use alloy_rpc_types_trace::geth::{CallConfig, GethDebugTracingOptions, GethTrace};
use sov_test_utils::Erc20;

use crate::evm::evm_test_helper::alloy_client;
use crate::evm::evm_test_helper::setup_test_rollup;
use crate::evm::evm_test_helper::EVM_EXTENSION;

#[tokio::test(flavor = "multi_thread")]
async fn debug_trace_pending_tx() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let client = alloy_client(rollup.http_addr);
    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    let mint_tx = usdc.mint(Address::ZERO, parse_ether("1")?).send().await?;
    rollup.pause_preferred_batches().await;

    let opts = GethDebugTracingOptions::call_tracer(CallConfig::default());
    let trace = client
        .debug_trace_transaction(*mint_tx.tx_hash(), opts.clone())
        .await?;

    // Verify the trace structure for the pending transaction
    match trace {
        GethTrace::CallTracer(call_frame) => {
            // Verify it's a CALL to the USDC contract
            assert_eq!(call_frame.typ, "CALL");
            assert_eq!(call_frame.to, Some(*usdc.address()));

            // Verify the function signature is mint(address,uint256) = 0x40c10f19
            assert!(call_frame.input.starts_with(&[0x40, 0xc1, 0x0f, 0x19]));

            // Verify execution succeeded (no error, no revert)
            assert!(call_frame.error.is_none());
            assert!(call_frame.revert_reason.is_none());

            // Verify gas was consumed (non-zero gas_used)
            assert!(call_frame.gas_used > 0);
        }
        _ => panic!("Expected CallTracer variant, got {trace:?}"),
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn debug_trace_pending_block() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let client = alloy_client(rollup.http_addr);
    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    let mint_tx = usdc.mint(Address::ZERO, parse_ether("1")?).send().await?;
    rollup.pause_preferred_batches().await;

    let opts = GethDebugTracingOptions::call_tracer(CallConfig::default());
    let traces = client
        .debug_trace_block_by_number(BlockNumberOrTag::Pending, opts.clone())
        .await?;

    // Should have 2 traces: deployment + mint
    assert_eq!(traces.len(), 2);

    // Second trace should be the mint transaction
    assert_eq!(traces[1].tx_hash().unwrap(), *mint_tx.tx_hash());

    // Verify the mint trace
    if let alloy_rpc_types_trace::geth::TraceResult::Success { result, .. } = &traces[1] {
        match result {
            GethTrace::CallTracer(call_frame) => {
                assert_eq!(call_frame.typ, "CALL");
                assert_eq!(call_frame.to, Some(*usdc.address()));
                assert!(call_frame.error.is_none());
            }
            _ => panic!("Expected CallTracer variant"),
        }
    } else {
        panic!("Expected Success variant");
    }

    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn debug_trace_block_by_number() -> anyhow::Result<()> {
    let rollup = setup_test_rollup(0, EVM_EXTENSION).await;
    rollup.wait_for_next_blocks(1).await;

    let client = alloy_client(rollup.http_addr);
    let usdc = Erc20::deploy(client.clone(), "Usdc".into(), "USDC".into()).await?;
    let mint_tx = usdc.mint(Address::ZERO, parse_ether("1")?).send().await?;
    rollup.wait_for_next_blocks(1).await; // Block nr 2 mined with 2 transactions

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
