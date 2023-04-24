use crate::{create_new_demo, data_generation::simulate_da_with_revert_msg};
use sov_app_template::{AppTemplate, Batch};
use sovereign_sdk::stf::StateTransitionFunction;

const SEQUENCER_DA_ADDRESS: [u8; 32] = [1; 32];
const LOCKED_AMOUNT: u64 = 200;
/*
#[test]
fn test_tx_revert() {
    let path = schemadb::temppath::TempPath::new();
    {
        let mut demo = create_new_demo(LOCKED_AMOUNT + 1, &path);

        demo.init_chain(());
        demo.begin_slot();

        let txs = simulate_da_with_revert_msg();

        demo.apply_batch(Batch { txs }, &SEQUENCER_DA_ADDRESS, None)
            .expect("Batch is valid");

        demo.end_slot();
    }
}
*/
