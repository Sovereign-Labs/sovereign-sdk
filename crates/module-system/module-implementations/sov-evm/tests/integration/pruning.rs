use crate::helpers::*;
use crate::runtime::S;
use sov_evm::Evm;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_test_utils::BatchTestCase;

#[test]
fn test_pruning() {
    // Adjust the global variables to speed up the test.
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_EVM_BLOCK_PRUNING_THRESHOLD", "5");
    std::env::set_var(
        "SOV_TEST_CONST_OVERRIDE_EVM_TRANSACTION_PRUNING_THRESHOLD",
        "10",
    );

    let block_pruning_threshold = config_value!("EVM_BLOCK_PRUNING_THRESHOLD");
    let tx_pruning_threshold = config_value!("EVM_TRANSACTION_PRUNING_THRESHOLD");

    let (mut runner, from, to) = setup();
    let value = 1;

    let mut nonce = 0;
    for b in 0..10 * block_pruning_threshold {
        let mut txs = Vec::new();

        // Each block contains differen number of txs.
        for _ in 0..(b + tx_pruning_threshold) {
            let transfer_tx = create_transfer_tx(nonce, &from, &to, value);
            txs.push(transfer_tx);
            nonce += 1;
        }
        runner.execute_batch(BatchTestCase {
            input: txs.into(),
            assert: Box::new(move |_ctx, state| {
                let evm = Evm::<S>::default();
                let blocks_len = evm.blocks.len(state).unwrap_infallible();
                let transactions_len = evm.transactions.len(state).unwrap_infallible();
                let receipts_len = evm.receipts.len(state).unwrap_infallible();

                assert!(blocks_len > 0);
                assert!(transactions_len > 0);
                assert!(receipts_len > 0);

                assert!(blocks_len <= block_pruning_threshold);
                assert!(transactions_len <= tx_pruning_threshold);
                assert!(receipts_len <= tx_pruning_threshold);
            }),
        });
    }
}
