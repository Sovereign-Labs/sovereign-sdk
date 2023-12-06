//! Workflows for transaction management

use std::path::Path;

use serde::Serialize;
use sov_modules_api::clap::{self, Subcommand};
use sov_modules_api::cli::CliWalletArg;
use sov_modules_api::DispatchCall;

use crate::wallet_state::WalletState;

#[derive(clap::Parser)]
/// Generate, sign, and send transactions
pub enum TransactionWorkflow<File: Subcommand, Json: Subcommand> {
    /// Import a transaction
    #[clap(subcommand)]
    Import(ImportTransaction<File, Json>),
    /// Delete the current batch of transactions
    Clean,
    /// Remove a single transaction from the current batch
    Remove {
        /// The index of the transaction to remove, starting from 0
        index: usize,
    },
    /// List the current batch of transactions
    List,
}

impl<File: Subcommand, Json: Subcommand> TransactionWorkflow<File, Json> {
    /// Run the transaction workflow
    pub fn run<RT: DispatchCall, C: sov_modules_api::Context, E1, E2>(
        self,
        wallet_state: &mut WalletState<RT::Decodable, C>,
        _app_dir: impl AsRef<Path>,
    ) -> Result<(), anyhow::Error>
    where
        File: CliWalletArg<RT, Error = E1>,
        Json: CliWalletArg<RT, Error = E2>,
        RT::Decodable: Serialize,
        E1: Into<anyhow::Error> + Send + Sync,
        E2: Into<anyhow::Error> + Send + Sync,
    {
        match self {
            TransactionWorkflow::Import(import_workflow) => import_workflow.run(wallet_state),
            TransactionWorkflow::List => {
                println!("Current batch:");
                println!(
                    "{}",
                    serde_json::to_string_pretty(&wallet_state.unsent_transactions)?
                );
                Ok(())
            }
            TransactionWorkflow::Clean => {
                wallet_state.unsent_transactions.clear();
                Ok(())
            }
            TransactionWorkflow::Remove { index } => {
                wallet_state.unsent_transactions.remove(index);
                Ok(())
            }
        }
    }
}
/// An argument passed as path to a file
#[derive(clap::Parser)]
pub struct FileArg {
    /// The path to the file
    #[arg(long, short)]
    pub path: String,
}

#[derive(clap::Subcommand)]
/// Import a pre-formatted transaction from a JSON file or as a JSON string
pub enum ImportTransaction<Json: Subcommand, File: Subcommand> {
    /// Import a transaction from a JSON file at the provided path
    #[clap(subcommand)]
    FromFile(Json),
    /// Provide a JSON serialized transaction directly as input
    #[clap(subcommand)]
    FromString(
        /// The JSON serialized transaction as a string.
        /// The expected format is: {"module_name": {"call_name": {"field_name": "field_value"}}}
        File,
    ),
}

impl<Json, File> ImportTransaction<Json, File>
where
    Json: Subcommand,
    File: Subcommand,
{
    /// Parse from a file or a json string
    pub fn run<RT: DispatchCall, C: sov_modules_api::Context, E1, E2>(
        self,
        wallet_state: &mut WalletState<RT::Decodable, C>,
    ) -> Result<(), anyhow::Error>
    where
        Json: CliWalletArg<RT, Error = E1>,
        File: CliWalletArg<RT, Error = E2>,
        RT::Decodable: Serialize,
        E1: Into<anyhow::Error> + Send + Sync,
        E2: Into<anyhow::Error> + Send + Sync,
    {
        let tx = match self {
            ImportTransaction::FromFile(file) => file
                .decode_call_from_readable()
                .map_err(Into::<anyhow::Error>::into)?,

            ImportTransaction::FromString(json) => json
                .decode_call_from_readable()
                .map_err(Into::<anyhow::Error>::into)?,
        };

        println!("Adding the following transaction to batch:");
        println!("{}", serde_json::to_string_pretty(&tx)?);

        wallet_state.unsent_transactions.push(tx);

        Ok(())
    }
}
