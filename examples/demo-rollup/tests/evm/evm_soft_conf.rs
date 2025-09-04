use super::evm_test_helper;
use crate::evm::evm_test_helper::setup;

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_soft_confirmations() -> anyhow::Result<()> {
    let (test_rollup, evm_client, _, _) = setup(0).await;

    let contract_address = evm_test_helper::deploy_contract_check(&evm_client)
        .await
        .unwrap();

    // Test: Pause the sequencer and verify that the transaction receipt has no assigned block hash,
    // since the block hash is not yet known.
    {
        test_rollup.pause_preferred_batches().await;

        let set_arg = 1;
        let set_value_req = evm_client.set_value(contract_address, set_arg).await;
        let tx_hash = set_value_req.tx_hash();
        let rec = evm_client.receipt(tx_hash).await.unwrap();

        assert!(rec.block_hash.is_none());
        let block_nr = evm_client.block_number().await;

        // Now we created a block and the block hash becomes available.
        test_rollup.resume_preferred_batches().await;

        test_rollup.wait_for_next_blocks(1).await;
        let rec = evm_client.receipt(tx_hash).await.unwrap();
        assert!(rec.block_hash.is_some());

        assert_eq!(rec.block_number.unwrap().as_u64(), block_nr + 1);
    }

    // Check that invalid trsnacations are rejected.
    {
        let req = evm_client.always_reverts(contract_address).await;

        let err_str = req.unwrap_err().to_string();
        assert!(err_str.contains("EVM execution error: Revert"));
    }

    Ok(())
}
