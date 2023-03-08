use sov_state::JmtStorage;
use sovereign_sdk::stf::StateTransitionFunction;
mod types;
fn main() {
    let mut demo = types::Demo {
        current_storage: JmtStorage::with_path("demo_datadir")
            .expect("Must be able to open datadir"),
    };
    demo.init_chain(());
    demo.begin_slot();
    demo.apply_batch(types::Batch { txs: vec![] }, &[1u8; 32], None)
        .expect("Batch is valid");

    demo.end_slot();
    println!("Hello, world!")
}
