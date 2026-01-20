use crate::helpers::*;
use crate::runtime::S;
use alloy_primitives::{B256, U64};
use alloy_rpc_types::{BlockNumberOrTag, BlockTransactions};
use sov_chain_state::ChainState;
use sov_evm::Evm;
use sov_modules_api::module::GasSpec;
use sov_modules_api::prelude::UnwrapInfallible;
use sov_modules_api::runtime::capabilities::BlockGasInfo;
use sov_modules_api::Spec;
use sov_rollup_interface::common::RollupHeight;
use sov_test_utils::BatchTestCase;

#[test]
fn test_fee_history_latest_projects_pending() {
    let (mut runner, from, to, _) = setup();
    let transfer = create_transfer_tx(0, &from, &to, 1).tx;
    let evm = Evm::<S>::default();
    let chain_state = ChainState::<S>::default();

    runner.execute_batch(BatchTestCase {
        input: vec![transfer].into(),
        assert: Box::new(move |_ctx, state| {
            let sealed_head = *evm.block_numbers(state).end();
            let fee_history = evm
                .fee_history(U64::from(1), BlockNumberOrTag::Latest, None, state)
                .unwrap();

            assert_eq!(fee_history.oldest_block, sealed_head + 1);
            assert_eq!(fee_history.gas_used_ratio, vec![0.75]);
            assert_eq!(fee_history.base_fee_per_gas.len(), 2);

            let gas_info = chain_state
                .historical_gas_info_at(RollupHeight::new(sealed_head), state)
                .unwrap_infallible()
                .unwrap_or_else(|| {
                    BlockGasInfo::new(S::initial_gas_limit(), S::initial_base_fee_per_gas())
                });
            let pending_base_fee = ChainState::<S>::compute_base_fee_per_gas(gas_info.clone(), 1);
            let pending_gas_limit = gas_info.gas_limit().clone();
            let assumed_gas_used = assumed_pending_gas_used::<S>(&pending_gas_limit);
            let pending_gas_info =
                BlockGasInfo::with_usage(pending_gas_limit, pending_base_fee, assumed_gas_used);
            let next_base_fee = ChainState::<S>::compute_base_fee_per_gas(pending_gas_info, 1);

            assert_eq!(
                fee_history.base_fee_per_gas[0],
                pending_base_fee.as_ref()[0].0
            );
            assert_eq!(fee_history.base_fee_per_gas[1], next_base_fee.as_ref()[0].0);
        }),
    });
}

#[test]
fn test_get_block_by_hash_pending_synthetic() {
    let (mut runner, from, to, _) = setup();
    let transfer = create_transfer_tx(0, &from, &to, 1).tx;
    let evm = Evm::<S>::default();

    runner.execute_batch(BatchTestCase {
        input: vec![transfer].into(),
        assert: Box::new(move |_ctx, state| {
            let pending_block = evm.pending_block(state);
            let tx_index = if pending_block.transactions.start < pending_block.transactions.end {
                pending_block.transactions.end.saturating_sub(1)
            } else {
                pending_block.transactions.end
            };
            let pending_hash = synthetic_block_hash(pending_block.header.number, tx_index);
            let block = evm
                .get_block_by_hash(pending_hash, Some(false), state)
                .unwrap()
                .unwrap();

            assert_eq!(block.header.hash, pending_hash);
            assert_eq!(block.header.inner.number, pending_block.header.number);

            match block.transactions {
                BlockTransactions::Hashes(hashes) => assert!(hashes.is_empty()),
                BlockTransactions::Full(_) => {
                    panic!("expected hashes-only block for pending lookup")
                }
                BlockTransactions::Uncle => {
                    panic!("unexpected uncle-only block for pending lookup")
                }
            }
        }),
    });
}

fn assumed_pending_gas_used<S: Spec>(gas_limit: &S::Gas) -> S::Gas {
    let gas_used: Vec<u64> = gas_limit
        .as_ref()
        .iter()
        .map(|limit| limit.saturating_mul(3) / 4)
        .collect();
    S::Gas::try_from(gas_used)
        .unwrap_or_else(|err| panic!("Failed to build assumed pending gas used: {err:?}"))
}

fn synthetic_block_hash(block_number: u64, tx_index: u64) -> B256 {
    let mut bytes = [0u8; 32];
    bytes[16..24].copy_from_slice(&block_number.to_be_bytes());
    bytes[24..32].copy_from_slice(&tx_index.to_be_bytes());
    B256::from(bytes)
}
