pub mod sha256;

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum BenchCmd {
    /// Run the SHA-256 prover-gas sweep.
    Sha256(sha256::Sha256Args),
}

impl BenchCmd {
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            BenchCmd::Sha256(args) => sha256::run(args),
        }
    }
}
