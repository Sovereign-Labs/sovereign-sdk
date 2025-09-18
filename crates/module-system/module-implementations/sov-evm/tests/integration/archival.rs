use crate::helpers::*;
use crate::runtime::RT;
use crate::runtime::S;
use alloy_primitives::FixedBytes;
use alloy_primitives::Log;
use alloy_primitives::U256;
use alloy_rpc_types::BlockTransactions;
use revm::Database;
use sov_evm::Evm;
use sov_modules_api::capabilities::RollupHeight;
use sov_modules_api::GasArray;
use sov_test_utils::TransactionType;
use sov_test_utils::{BatchTestCase, SimpleStorageContract};
use sov_test_utils::{TransactionTestCase, TEST_DEFAULT_USER_BALANCE};

#[test]
fn test_archival() {
    let (mut runner, from, to) = setup();

    let value = 1;
    let transfer_tx = create_transfer_tx(0, &from, &to, value).tx;

    let to_addr = to.address();

    let evm = Evm::<S>::default();
    runner.execute_transaction(TransactionTestCase {
        input: transfer_tx,
        assert: Box::new(move |ctx, state| {
            let mut db = evm.get_db(state);
            let from_acc = db.basic(from.address()).unwrap().unwrap();
            let to_acc = db.basic(to.address()).unwrap().unwrap();
            // The only balance changes should be from the trasfer itself and not from gas as it's disabled in SovEvm
            assert_eq!(
                from_acc.balance,
                TEST_DEFAULT_USER_BALANCE.0 - value - ctx.gas_value_used.0
            );
            assert_eq!(to_acc.balance, value);
        }),
    });

    let evm = Evm::<S>::default();
    let (block, balance) = runner.query_state(|state| {
        let evm = Evm::<S>::default();
        (
            evm.get_block_by_number(Some("latest".to_string()), None, state)
                .unwrap()
                .unwrap(),
            evm.get_balance(to_addr, None, state).unwrap(),
        )
    });

    let block_nr = block.header.number;

    let balance2 = runner.query_state(|state| {
        let evm = Evm::<S>::default();
        let mut archival_state = state
            .get_archival_state(RollupHeight::new(block_nr))
            .unwrap();
        evm.get_balance(to_addr, None, &mut archival_state).unwrap()
    });

    assert_eq!(balance, balance2);
}
