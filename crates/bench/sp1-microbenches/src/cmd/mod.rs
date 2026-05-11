pub mod sha256;

use std::fs;
use std::path::{Path, PathBuf};

use clap::Subcommand;

#[derive(Subcommand, Debug)]
pub enum BenchCmd {
    /// Run the SHA-256 prover-gas sweep.
    Sha256(sha256::Sha256Args),
}

impl BenchCmd {
    /// Explicit `--out` path if the user passed one. Borrowed so we can use it
    /// before `self` is consumed by `run`.
    pub fn out(&self) -> Option<&Path> {
        match self {
            BenchCmd::Sha256(args) => args.out.as_deref(),
        }
    }

    pub fn run(self) -> anyhow::Result<()> {
        let explicit_out: Option<PathBuf> = self.out().map(PathBuf::from);

        let output = match self {
            BenchCmd::Sha256(args) => sha256::run(args)?,
        };

        let path = explicit_out.unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("reports")
                .join(&output.default_filename)
        });
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &output.markdown)?;

        println!("{}", output.summary);
        println!("report written to: {}", path.display());
        Ok(())
    }
}
