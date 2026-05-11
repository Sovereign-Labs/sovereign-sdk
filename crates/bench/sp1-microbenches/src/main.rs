use clap::{Parser, Subcommand};
use sp1_microbenches::cmd::sha256;

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
}

impl BenchCmd {
    fn run(self) -> anyhow::Result<()> {
        match self {
            BenchCmd::Sha256(args) => sha256::run(args),
        }
    }
}

fn main() -> anyhow::Result<()> {
    Cli::parse().cmd.run()
}
