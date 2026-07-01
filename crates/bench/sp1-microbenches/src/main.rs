use clap::{Parser, Subcommand};
use sp1_microbenches::cmd::{celestia, ed25519, sha256};

#[derive(Parser, Debug)]
#[command(
    name = "sp1-microbenches",
    about = "ZK gas calibration microbenchmarks"
)]
struct Cli {
    #[command(subcommand)]
    cmd: BenchCmd,
}

#[derive(Subcommand, Debug)]
enum BenchCmd {
    /// Run the SHA-256 prover-gas sweep.
    Sha256(sha256::Sha256Args),
    /// Run the ed25519 signature-verification prover-gas sweep.
    Ed25519(ed25519::Ed25519Args),
    /// Run the Celestia verifier prover-gas bench.
    Celestia(celestia::CelestiaArgs),
}

impl BenchCmd {
    fn run(self) -> anyhow::Result<()> {
        match self {
            BenchCmd::Sha256(args) => sha256::run(args),
            BenchCmd::Ed25519(args) => ed25519::run(args),
            BenchCmd::Celestia(args) => celestia::run(args),
        }
    }
}

fn main() -> anyhow::Result<()> {
    Cli::parse().cmd.run()
}
