use clap::Parser;
use sp1_microbenches::cmd::BenchCmd;

#[derive(Parser, Debug)]
#[command(
    name = "sp1-microbenches",
    about = "ZK gas calibration microbenchmarks"
)]
struct Cli {
    #[command(subcommand)]
    cmd: BenchCmd,
}

fn main() -> anyhow::Result<()> {
    Cli::parse().cmd.run()
}
