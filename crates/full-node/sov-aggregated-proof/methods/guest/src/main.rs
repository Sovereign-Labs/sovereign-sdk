use hex::FromHex;
use risc0_zkp::core::digest::Digest;
use risc0_zkvm::{guest::env, serde};
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::Spec;
use sov_risc0_adapter::Risc0;
use sov_rollup_interface::execution_mode::Zk;
use sov_rollup_interface::zk::StateTransitionPublicData;
use sov_state::Storage;

type S = DefaultSpec<MockDaSpec, Risc0, MockZkvm, Zk>;

fn main() {
    let method_id =
        Digest::from_hex("665839999d6b39fff2bfce839e709d4eb0eb75cdcda76219729fb81b5fd381ca")
            .unwrap();

    let witnesses: Vec<
        StateTransitionPublicData<
            <S as Spec>::Address,
            MockDaSpec,
            <<S as Spec>::Storage as Storage>::Root,
        >,
    > = env::read();

    for witness in witnesses.into_iter() {
        env::verify(method_id, &serde::to_vec(&witness).unwrap()).unwrap();
    }
}
