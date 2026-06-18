//! SP1 light-client verification for the demo rollup.
//!
//! Derives the inner/outer verification keys from local guest ELFs, then fetches
//! and verifies the node's latest aggregated proof against them.

use std::path::PathBuf;

use sov_demo_rollup::MockSp1RollupSpec;
use sov_modules_api::execution_mode::Native;
use sov_modules_api::{Spec, Storage};
use sov_node_client::NodeClient;
use sov_rollup_interface::zk::ZkLightClient;
use sov_sp1_adapter::light_client::Sp1LightClient;

/// Public-value types committed to by the SP1 demo rollup, mirroring the spec
/// the node runs with ([`MockSp1DemoRollup`](sov_demo_rollup::MockSp1DemoRollup)).
type Address = <MockSp1RollupSpec<Native> as Spec>::Address;
type Da = <MockSp1RollupSpec<Native> as Spec>::Da;
type Root = <<MockSp1RollupSpec<Native> as Spec>::Storage as Storage>::Root;

/// Default location of the SP1 state-transition (inner) guest ELF, relative to
/// the `examples/demo-rollup` working directory used to run the node.
const DEFAULT_SP1_INNER_ELF: &str = "provers/sp1/guest-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-demo-prover-guest-mock-sp1";

/// Default location of the SP1 aggregation (outer) guest ELF, relative to the
/// `examples/demo-rollup` working directory used to run the node.
const DEFAULT_SP1_OUTER_ELF: &str = "provers/sp1/guest-aggregation-mock/target/elf-compilation/riscv64im-succinct-zkvm-elf/release/sov-aggregated-proof-program";

/// Fetches the node's latest aggregated proof from the node at `node_url`,
/// verifies it with an [`Sp1LightClient`], and prints the verified public values.
///
/// The ELF paths fall back to [`DEFAULT_SP1_INNER_ELF`] / [`DEFAULT_SP1_OUTER_ELF`]
/// when left unset.
pub async fn verify_latest_aggregated_proof(
    node_url: &str,
    inner_elf_path: Option<PathBuf>,
    outer_elf_path: Option<PathBuf>,
) -> anyhow::Result<()> {
    let inner_elf_path = inner_elf_path.unwrap_or_else(|| PathBuf::from(DEFAULT_SP1_INNER_ELF));
    let outer_elf_path = outer_elf_path.unwrap_or_else(|| PathBuf::from(DEFAULT_SP1_OUTER_ELF));

    let light_client = Sp1LightClient::new(inner_elf_path, outer_elf_path).await?;
    let proof = NodeClient::new_unchecked(node_url)
        .fetch_latest_aggregated_proof()
        .await?;
    let public_data = light_client.verify_aggregated_proof::<Address, Da, Root>(proof)?;
    println!("Proof verified successfully. Public values:\n{public_data:#?}");

    Ok(())
}
