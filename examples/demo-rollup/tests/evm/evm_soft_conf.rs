use super::evm_test_helper;
use crate::evm::evm_test_helper::{setup_with_simple_storage, EVM_EXTENSION};

#[tokio::test(flavor = "multi_thread")]
async fn evm_test_soft_confirmations() -> anyhow::Result<()> {
    let (test_rollup, evm_client, _) = setup_with_simple_storage(0, EVM_EXTENSION).await;

    let contract_address = evm_test_helper::deploy_contract_check(&evm_client)
        .await
        .unwrap();

    test_rollup.wait_for_next_blocks(1).await;

    // Test: Pause the sequencer and verify that the transaction receipt has no assigned block hash,
    // since the block hash is not yet known.
    {
        test_rollup.pause_preferred_batches().await;

        // Verify the `pending_block and latest_block`` assertions before inserting the transaction.
        {
            let latest_block = evm_client
                .eth_get_block_by_number(Some("latest".to_string()))
                .await;

            let pending_block = evm_client
                .eth_get_block_by_number(Some("pending".to_string()))
                .await;

            assert_eq!(latest_block, pending_block);
            assert!(pending_block.transactions.is_empty());
        }

        let set_arg = 1;
        let tx_hash = evm_client.set_value(contract_address, set_arg).await;

        let expected_block_nr = evm_client.block_number().await + 1;

        // Verify the `receipt & transaction` asserts.
        {
            let rec = evm_client.receipt(tx_hash).await.unwrap();
            let tx = evm_client.transaction(tx_hash).await.unwrap();

            assert!(rec.block_hash.is_none());
            assert!(tx.block_hash.is_none());

            assert_eq!(rec.block_number.unwrap(), expected_block_nr);
            assert_eq!(tx.block_number.unwrap(), expected_block_nr);
        }

        // Verify the `pending_block` asserts after inserting the transaction.
        {
            let pending_block = evm_client
                .eth_get_block_by_number(Some("pending".to_string()))
                .await;

            assert_eq!(pending_block.header.number, expected_block_nr);
            assert_eq!(
                pending_block.transactions.hashes().collect::<Vec<_>>(),
                vec![tx_hash]
            );
            assert_ne!(pending_block.header.timestamp, 0);
        }

        // Now we created a block and the block hash becomes available.
        test_rollup.resume_preferred_batches().await;

        test_rollup.wait_for_next_blocks(1).await;

        {
            let rec = evm_client.receipt(tx_hash).await.unwrap();
            let tx = evm_client.transaction(tx_hash).await.unwrap();

            assert_eq!(rec.block_hash, tx.block_hash);

            assert_eq!(rec.block_number.unwrap(), expected_block_nr);
            assert_eq!(tx.block_number.unwrap(), expected_block_nr);
        }
    }

    // Check that invalid trsnacations are rejected.
    {
        let req = evm_client.always_reverts(contract_address).await;

        let err_str = req.unwrap_err().to_string();
        assert!(err_str.contains("EVM execution error: Revert"));
    }

    Ok(())
}
