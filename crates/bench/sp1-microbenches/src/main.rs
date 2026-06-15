use clap::{Parser, Subcommand};
use sp1_microbenches::cmd::{borsh, ed25519, sha256, storage};

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
    /// Run the borsh deserialization prover-gas sweeps.
    Borsh(borsh::BorshArgs),
    /// Run the NOMT storage proof-verification prover-gas sweep.
    Storage(storage::StorageArgs),
}

impl BenchCmd {
    fn run(self) -> anyhow::Result<()> {
        match self {
            BenchCmd::Sha256(args) => sha256::run(args),
            BenchCmd::Ed25519(args) => ed25519::run(args),
            BenchCmd::Borsh(args) => borsh::run(args),
            BenchCmd::Storage(args) => storage::run(args),
        }
    }
}

fn main() -> anyhow::Result<()> {
    Cli::parse().cmd.run()
}
