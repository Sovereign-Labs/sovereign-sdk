use super::genesis_tests::{get_evm, TEST_CONFIG};
use crate::evm::transaction::BlockEnv;

#[test]
fn begin_slot_hook_creates_pending_block() {
    let (evm, mut working_set) = get_evm(&TEST_CONFIG);
    evm.begin_slot_hook([5u8; 32], &mut working_set);
    let pending_block = evm.pending_block.get(&mut working_set).unwrap();
    assert_eq!(
        pending_block,
        BlockEnv {
            number: 1,
            coinbase: [3u8; 20],
            timestamp: {
                let mut a = [0u8; 32];
                a[0] = 52;
                a
            },
            prevrandao: Some([5u8; 32]),
            basefee: {
                let mut a = [0u8; 32];
                a[0] = 62;
                a
            },
            gas_limit: 30000000,
        }
    );
}
