use std::str::FromStr;

use clap::Parser;
use sov_demo_rollup::{new_rollup_with_celestia_da, new_rollup_with_mock_da};
use sov_risc0_adapter::host::Risc0Host;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

#[cfg(test)]
mod test_rpc;

/// Main demo runner. Initialize a DA chain, and starts a demo-rollup using the config provided
/// (or a default config if not provided). Then start checking the blocks sent to the DA layer in
/// the main event loop.

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// The data layer type.
    #[arg(long, default_value = "celestia")]
    da_layer: String,

    /// The path to the rollup config.
    #[arg(long, default_value = "rollup_config.toml")]
    rollup_config_path: String,
}

/*
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Initializing logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_str("info,sov_sequencer=warn").unwrap())
        .init();

    let args = Args::parse();
    let rollup_config_path = args.rollup_config_path.as_str();

    match args.da_layer.as_str() {
        "mock" => {
            let rollup = new_rollup_with_mock_da::<Risc0Host<'static>>(rollup_config_path, None)?;
            rollup.run().await
        }
        "celestia" => {
            let rollup =
                new_rollup_with_celestia_da::<Risc0Host<'static>>(rollup_config_path, None).await?;
            rollup.run().await
        }
        da => panic!("DA Layer not supported: {}", da),
    }
}*/

pub mod sov_api {

    pub trait GasUnit {
        fn from_str(s: &str) -> Self;
        fn value(price: Self) -> u64;
    }

    pub struct TupleGasUnit {
        zk_gas: u64,
        native_gas: u64,
    }

    impl GasUnit for TupleGasUnit {
        fn from_str(s: &str) -> Self {
            todo!()
        }

        fn value(price: Self) -> u64 {
            todo!()
        }
    }

    pub trait Context {
        type GasUnit: GasUnit;
    }

    pub struct DefaultContext;

    impl Context for DefaultContext {
        type GasUnit = TupleGasUnit;
    }

    pub fn parse_toml_file_to_lines(s: &str) -> Vec<String> {
        todo!()
    }
}

mod some_sov_module {
    use super::sov_api::{parse_toml_file_to_lines, Context, GasUnit};

    pub struct GasConfig<GU: GasUnit> {
        pub complex_math_operation: GU,
        pub some_other_operation: GU,
    }

    // This would be macro generates
    pub fn from_toml<C: Context>(t: &str) -> GasConfig<C::GasUnit> {
        let strs = parse_toml_file_to_lines(t);
        let complex_math_operation = C::GasUnit::from_str(&strs[0]);
        let some_other_operation = C::GasUnit::from_str(&strs[1]);

        GasConfig {
            complex_math_operation,
            some_other_operation,
        }
    }
}

mod runtime {}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    todo!();
}
