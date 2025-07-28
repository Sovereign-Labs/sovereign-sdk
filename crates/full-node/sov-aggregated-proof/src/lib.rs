use borsh::from_slice;
use risc0_zkvm::Receipt;
use std::fs;

use risc0::{PROOF_AGGREGATION_ELF, PROOF_AGGREGATION_ID}; //::AGGREGATION_ELF;
use risc0_zkvm::default_prover;
use risc0_zkvm::ExecutorEnv;
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::MockZkvm;
use sov_modules_api::default_spec::DefaultSpec;
use sov_modules_api::Spec;
use sov_risc0_adapter::Risc0;
use sov_rollup_interface::execution_mode::Zk;
use sov_rollup_interface::zk::StateTransitionPublicData;
use sov_state::Storage;

fn read_receipt(file_path: &str) -> anyhow::Result<Receipt> {
    let data: Vec<u8> = fs::read(file_path).unwrap();
    Ok(from_slice::<Receipt>(&data)?)
}

type S = DefaultSpec<MockDaSpec, Risc0, MockZkvm, Zk>;

pub fn check_receipts(file_paths: Vec<&str>) -> anyhow::Result<()> {
    let receipts = file_paths
        .into_iter()
        .map(|file_path| read_receipt(&file_path))
        .collect::<Result<Vec<_>, _>>()?;

    let mut env = ExecutorEnv::builder();

    let mut witnesses = Vec::default();
    for r in receipts.into_iter() {
        env.add_assumption(r.clone());
        witnesses.push(decode(r));
    }

    let env = env.write(&witnesses).unwrap().build().unwrap();

    let receipt = default_prover()
        .prove(env, PROOF_AGGREGATION_ELF)
        .unwrap()
        .receipt;
    receipt.verify(PROOF_AGGREGATION_ID).unwrap();

    Ok(())
}

fn decode(
    rec: Receipt,
) -> StateTransitionPublicData<
    <S as Spec>::Address,
    MockDaSpec,
    <<S as Spec>::Storage as Storage>::Root,
> {
    rec.journal.decode().expect(
        "Journal output should deserialize into the same types (& order) that it was written",
    )
}
