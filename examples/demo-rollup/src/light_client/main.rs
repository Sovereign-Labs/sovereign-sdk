//! A minimal light client for the demo rollup.
//!
//! It fetches the latest verified (aggregated / "outer") proof from a running
//! node, derives the outer verification key for the selected zkVM, verifies the
//! proof, and prints the verified public values. The verification logic lives in
//! [`sov_demo_rollup::light_client`]; this binary is just a CLI wrapper that
//! dispatches to the right [`ZkLightClient`] implementation based on `--zk-vm`.
//!
//! The `--zk-vm` value must match the VM the node runs. For example, against a
//! node started with `cargo run --bin sov-demo-rollup -- --zk-vm sp1`, run the
//! light client with `cargo run --bin light-client -- --zk-vm sp1`.

use std::path::PathBuf;

use clap::Parser;
use sov_demo_rollup::{read_mock_code_commitments_from_env, MockSp1RollupSpec, SupportedZkVm};
use sov_mock_da::MockDaSpec;
use sov_mock_zkvm::light_client::MockLightClient;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{CodeCommitmentTrait, Spec, Storage};
use sov_rollup_interface::zk::ZkLightClient;
use sov_sp1_adapter::light_client::Sp1LightClient;

/// Concrete spec of the mock-DA demo rollup. The mock and SP1 specs share the
/// same address and state-root types, so a single alias fixes the public-value
/// shape the prover committed to regardless of the selected zkVM.
type LightClientSpec = MockSp1RollupSpec<Native>;
type Address = <LightClientSpec as Spec>::Address;
type Root = <<LightClientSpec as Spec>::Storage as Storage>::Root;

/// Default location of the state-transition (inner) guest ELF, relative to the
/// `examples/demo-rollup` working directory used to run the node.
const DEFAULT_INNER_ELF: &str = "provers/sp1/guest-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-demo-prover-guest-mock-sp1";

/// Default location of the aggregation (outer) guest ELF, relative to the
/// `examples/demo-rollup` working directory used to run the node.
const DEFAULT_OUTER_ELF: &str = "provers/sp1/guest-aggregation-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-aggregated-proof-program";

#[derive(Parser, Debug)]
#[command(author, version, about = "Light client for the demo rollup", long_about = None)]
struct Args {
    /// The zk VM the node runs on.
    #[arg(long, default_value = "mock")]
    zk_vm: SupportedZkVm,

    /// Base URL of the rollup node's REST API.
    #[arg(long, default_value = "http://127.0.0.1:12346")]
    node_url: String,

    /// Path to the inner (state-transition) ELF used to derive the SP1
    /// verification key. Ignored when `--zk-vm mock`.
    #[arg(long, default_value = DEFAULT_INNER_ELF)]
    inner_elf_path: PathBuf,

    /// Path to the outer (aggregation-circuit) ELF used to derive the SP1
    /// verification key. Ignored when `--zk-vm mock`.
    #[arg(long, default_value = DEFAULT_OUTER_ELF)]
    outer_elf_path: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let url = format!(
        "{}/ledger/aggregated-proofs/latest",
        args.node_url.trim_end_matches('/')
    );

    let public_data = match args.zk_vm {
        SupportedZkVm::Sp1 => {
            Sp1LightClient::new(args.inner_elf_path, args.outer_elf_path)
                .await?
                .fetch_and_verify_latest_aggregated_proof::<Address, MockDaSpec, Root>(&url)
                .await?
        }
        SupportedZkVm::Mock => {
            let (inner_code_commitment, outer_code_commitment) =
                read_mock_code_commitments_from_env();
            MockLightClient::new(inner_code_commitment.to_hash(), outer_code_commitment)
                .fetch_and_verify_latest_aggregated_proof::<Address, MockDaSpec, Root>(&url)
                .await?
        }
    };

    println!("Proof verified successfully. Public values:\n{public_data:#?}");

    Ok(())
}
