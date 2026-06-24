//! A minimal light client for the demo rollup.
//!
//! It fetches the latest verified (aggregated / "outer") proof from a running
//! node, derives the outer verification key for the selected zkVM, verifies the
//! proof, and prints the verified public values. Fetching and verifying go
//! through [`sov_node_client::NodeClient::fetch_and_verify_latest_aggregated_proof`];
//! this binary is just a CLI wrapper that dispatches to the right
//! [`ZkLightClient`](sov_rollup_interface::zk::ZkLightClient) implementation based
//! on `--zk-vm`.
//!
//! The `--zk-vm` value must match the VM the node runs. For example, against a
//! node started with `cargo run --bin sov-demo-rollup -- --zk-vm sp1`, run the
//! light client with `cargo run --bin light-client -- --zk-vm sp1`.

mod mock;
mod sp1;

use std::path::PathBuf;

use clap::Parser;
use sov_demo_rollup::SupportedZkVm;

#[derive(Parser, Debug)]
#[command(author, version, about = "Light client for the demo rollup", long_about = None)]
struct Args {
    /// The zk VM the node runs on.
    #[arg(long, default_value = "mock")]
    zk_vm: SupportedZkVm,

    /// Base URL of the rollup node's REST API.
    #[arg(long, default_value = "http://127.0.0.1:12346")]
    node_url: String,

    /// Path to the inner (state-transition) ELF.
    #[arg(long)]
    inner_elf_path: Option<PathBuf>,

    /// Path to the outer (aggregation-circuit) ELF.
    #[arg(long)]
    outer_elf_path: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    let node_url = args.node_url.trim_end_matches('/');

    // The verified public-value shape depends on the spec the node runs, which is
    // selected by `--zk-vm` (just as the node binary picks its rollup blueprint).
    // Each module decodes against the matching spec's types.
    match args.zk_vm {
        SupportedZkVm::Sp1 => {
            sp1::verify_latest_aggregated_proof(node_url, args.inner_elf_path, args.outer_elf_path)
                .await?;
        }
        SupportedZkVm::Mock => mock::verify_latest_aggregated_proof(node_url).await?,
    }

    Ok(())
}
