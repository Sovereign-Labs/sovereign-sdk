//! Workflows for transaction management

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_api::clap::{self, Args, Subcommand};

use crate::wallet_state::WalletState;

#[derive(clap::Parser)]
/// Generate, sign, and send transactions
pub enum TransactionWorkflow<RT: sov_modules_api::CliWallet>
where
    RT::CliStringRepr: clap::Subcommand,
{
    /// Parse a transaction from the command line and add it to the current batch
    #[clap(subcommand)]
    Generate(RT::CliStringRepr),
    /// Import a transaction from a JSON file or as a JSON string
    Import(TransactionSubcommand<ImportTransaction>),
    /// List the current batch of transactions
    List,
    // TODO: Add `send` and `generate_schema` subcommands/
    // TODO: design and implement batch management (remove tx, drop batch, etc.)
}

impl<RT: sov_modules_api::CliWallet> TransactionWorkflow<RT>
where
    RT::Decodable: Serialize + DeserializeOwned,
    RT::CliStringRepr: clap::Subcommand,
{
    /// Run the transaction workflow
    pub fn run<C: sov_modules_api::Context>(
        self,
        wallet_state: &mut WalletState<RT::Decodable, C>,
        _app_dir: impl AsRef<Path>,
    ) -> Result<(), anyhow::Error> {
        match self {
            TransactionWorkflow::Generate(subcommand) => {
                // let TransactionSubcommand { args, inner } = subcommand;
                let tx: RT::Decodable = subcommand.into();
                println!("Adding the following transaction to batch:");
                println!("{}", serde_json::to_string_pretty(&tx)?);
                wallet_state.unsent_transactions.push(tx);
            }
            TransactionWorkflow::Import(subcommand) => {
                let TransactionSubcommand { args: _, inner } = subcommand;
                let tx = match inner {
                    ImportTransaction::FromFile { path } => {
                        let tx = std::fs::read_to_string(path)?;
                        serde_json::from_str(&tx)?
                    }
                    ImportTransaction::FromString { json } => serde_json::from_str(&json)?,
                };
                println!("Adding the following transaction to batch:");
                println!("{}", serde_json::to_string_pretty(&tx)?);
                wallet_state.unsent_transactions.push(tx);
            }
            TransactionWorkflow::List => {
                println!("Current batch:");
                println!(
                    "{}",
                    serde_json::to_string_pretty(&wallet_state.unsent_transactions)?
                );
            }
        }

        Ok(())
    }
}

#[derive(clap::Subcommand)]
/// Import a pre-formatted transaction from a JSON file or as a JSON string
pub enum ImportTransaction {
    /// Import a transaction from a JSON file at the provided path
    #[command(arg_required_else_help(true))]
    FromFile {
        /// The expected format of the file contents is: {"module_name": {"call_name": {"field_name": "field_value"}}}
        path: PathBuf,
    },
    /// Provide a JSON serialized transaction directly as input
    #[command(arg_required_else_help(true))]
    FromString {
        /// The JSON serialized transaction as a string.
        /// The expected format is: {"module_name": {"call_name": {"field_name": "field_value"}}}
        json: String,
    },
}

/// A wrapper around a subcommand that also includes the optional global arguments
#[derive(clap::Parser)]
pub struct TransactionSubcommand<S: Subcommand> {
    /// The optional arguments
    #[clap(flatten)]
    pub args: OptionalArgs,
    /// The inner subcommand
    #[clap(subcommand)]
    pub inner: S,
}

/// The optional arguments for the transaction workflow
#[derive(Debug, Args)]
pub struct OptionalArgs {
    #[clap(short, long, global = true, default_value_t = false)]
    send_transactions: bool,
}
