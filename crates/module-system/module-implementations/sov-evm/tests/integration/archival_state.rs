use sov_evm::Evm;

use crate::helpers::{create_transfer_tx, setup};
use crate::runtime::S;

#[test]
fn test_state_at_different_depth_is_accessible() {
    let (mut runner, from, to, _) = setup();

    let evm = Evm::<S>::default();
    for tx_idx in 0..=1 {
        let transfer_tx = create_transfer_tx(tx_idx, &from, &to, 1).tx;
        runner.execute(transfer_tx);
    }
    runner.query_visible_state(|state| {
        let mut balance = |block: Option<&str>| {
            evm.get_balance(to.address(), block.map(Into::into), state)
                .unwrap()
        };
        assert_eq!(balance(None), 2);
        assert_eq!(balance(Some("latest")), 2);
        assert_eq!(balance(Some("pending")), 2);
        assert_eq!(balance(Some("0x00")), 0);
        assert_eq!(balance(Some("0x01")), 1);
        assert_eq!(balance(Some("0x02")), 2);
    });
}
