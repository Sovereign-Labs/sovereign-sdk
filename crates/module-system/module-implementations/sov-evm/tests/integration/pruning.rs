use crate::helpers::*;
use crate::runtime::S;
use alloy_consensus::BlockHeader;
use sov_evm::Evm;
use sov_modules_api::macros::config_value;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::ApiStateAccessor;
use sov_test_utils::BatchTestCase;

const TX_COUNT_PER_BLOCK: u64 = 20;

fn check_blocks(block_start: u64, block_end: u64, evm: &Evm<S>, state: &mut ApiStateAccessor<S>) {
    let blocks_range = evm.block_numbers(state);
    assert_eq!(*blocks_range.start(), block_start);
    assert_eq!(*blocks_range.end(), block_end);

    for block_number in block_start..=block_end {
        let block = evm.blocks.get(&block_number, state).unwrap().unwrap();
        assert_eq!(block.header().number(), block_number);
    }
}

fn check_transaction(index: u64, evm: &Evm<S>, state: &mut ApiStateAccessor<S>) -> Option<()> {
    let tx = evm.transaction(index, state)?;
    evm.transaction_hashes
        .get(tx.signed_transaction().hash(), state)
        .unwrap_infallible()?;
    evm.receipt(index, state)?;
    Some(())
}

#[test]
fn test_pruning() {
    // Adjust the global variables to speed up the test.
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_EVM_BLOCK_PRUNING_THRESHOLD", "5");
    let block_pruning_threshold = config_value!("EVM_BLOCK_PRUNING_THRESHOLD");

    let (mut runner, from, to, _) = setup();
    let value = 1;
    let mut nonce = 0;

    // The genesis is already included, so block counting begins at 1.
    for block_number in 1..block_pruning_threshold {
        let mut txs = Vec::new();
        for _ in 0..TX_COUNT_PER_BLOCK {
            let transfer_tx = create_transfer_tx(nonce, &from, &to, value).tx;
            txs.push(transfer_tx);
            nonce += 1;
        }

        let evm = Evm::<S>::default();
        runner.execute_batch(BatchTestCase {
            input: txs.into(),
            assert: Box::new(move |_ctx, state| {
                check_blocks(0, block_number, &evm, state);

                for index in 0..block_number * TX_COUNT_PER_BLOCK {
                    assert!(check_transaction(index, &evm, state).is_some());
                }
            }),
        });
    }

    // Create empty batch and evict genesis.
    let evm = Evm::<S>::default();
    runner.execute_batch(BatchTestCase {
        input: vec![].into(),
        assert: Box::new(move |_ctx, state| {
            check_blocks(1, block_pruning_threshold, &evm, state);

            // Since the genesis block had no transactions, none were removed.
            for index in 0..(block_pruning_threshold - 1) * TX_COUNT_PER_BLOCK {
                assert!(check_transaction(index, &evm, state).is_some());
            }
        }),
    });

    let evm = Evm::<S>::default();
    let transfer_tx = create_transfer_tx(nonce, &from, &to, value).tx;

    // Create another batch with a single transfer.
    runner.execute_batch(BatchTestCase {
        input: vec![transfer_tx].into(),
        assert: Box::new(move |_ctx, state| {
            check_blocks(2, block_pruning_threshold + 1, &evm, state);

            // The first non empty block was pruned so we should not see any corresponding transaction.
            let mut index = 0;
            for _ in 0..TX_COUNT_PER_BLOCK {
                assert!(evm.transaction(index, state).is_none());
                assert!(evm.receipt(index, state).is_none());
                index += 1;
            }

            // Transactions for all the other blocks are still in the state.
            for _ in TX_COUNT_PER_BLOCK..(block_pruning_threshold - 1) * TX_COUNT_PER_BLOCK + 1 {
                assert!(check_transaction(index, &evm, state).is_some());
                index += 1;
            }

            // There are no additional transactions
            assert!(evm.transaction(index, state).is_none());
            assert!(evm.receipt(index, state).is_none());
        }),
    });
}

#[test]
fn test_pruning_tolerates_missing_receipt_fees() {
    // Adjust the global variables to speed up the test.
    std::env::set_var("SOV_TEST_CONST_OVERRIDE_EVM_BLOCK_PRUNING_THRESHOLD", "5");
    let block_pruning_threshold = config_value!("EVM_BLOCK_PRUNING_THRESHOLD");

    let (mut runner, from, to, _) = setup();
    let value = 1;
    let mut nonce = 0;

    // Fill enough blocks to start pruning.
    for _ in 1..block_pruning_threshold {
        let transfer_tx = create_transfer_tx(nonce, &from, &to, value).tx;
        nonce += 1;

        runner.execute_batch(BatchTestCase {
            input: vec![transfer_tx].into(),
            assert: Box::new(move |_ctx, _state| {}),
        });
    }

    // Evict genesis block and then remove one receipt_fees entry to simulate
    // transactions that were stored before the accessory map was introduced.
    let evm = Evm::<S>::default();
    runner.execute_batch(BatchTestCase {
        input: vec![].into(),
        assert: Box::new(move |_ctx, state| {
            check_blocks(1, block_pruning_threshold, &evm, state);

            assert!(evm.transaction(0, state).is_some());
            assert!(evm.receipt(0, state).is_some());
            assert!(evm.receipt_fee(0, state).is_some());

            let removed = evm.receipt_fees.remove(&0, state).unwrap_infallible();
            assert!(removed.is_some());
            assert!(evm.receipt_fee(0, state).is_none());
        }),
    });

    // Prune the first non-empty block. This used to panic because prune_tx
    // expected receipt_fees to always exist.
    let evm = Evm::<S>::default();
    let transfer_tx = create_transfer_tx(nonce, &from, &to, value).tx;
    runner.execute_batch(BatchTestCase {
        input: vec![transfer_tx].into(),
        assert: Box::new(move |_ctx, state| {
            check_blocks(2, block_pruning_threshold + 1, &evm, state);

            // First non-empty block was pruned.
            assert!(evm.transaction(0, state).is_none());
            assert!(evm.receipt(0, state).is_none());
            assert!(evm.receipt_fee(0, state).is_none());
        }),
    });
}
