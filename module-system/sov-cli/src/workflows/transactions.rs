//! Workflows for transaction management

use std::path::Path;

use anyhow::Context;
use demo_stf::runtime::{JsonStringArg, RuntimeSubcommand};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::error;
use sov_modules_api::clap::{self, Args, Subcommand};
use sov_modules_api::CliWallet;

use crate::wallet_state::WalletState;

#[derive(clap::Parser)]
/// Generate, sign, and send transactions
pub enum TransactionWorkflow<T: Subcommand> {
    /// Import a transaction
    #[clap(subcommand)]
    Import(T),
}

impl<T: Subcommand> TransactionWorkflow<T> {
    /// Run the transaction workflow
    pub fn run<RT: CliWallet, C: sov_modules_api::Context, E1, E2, U>(
        self,
        wallet_state: &mut WalletState<RT::Decodable, C>,
        _app_dir: impl AsRef<Path>,
    ) -> Result<(), anyhow::Error>
    where
        T: TryInto<RT::CliStringRepr<U>, Error = E1>,
        RT::CliStringRepr<U>: TryInto<RT::Decodable, Error = E2>,
        RT::Decodable: Serialize,
        E1: Into<anyhow::Error> + Send + Sync,
        E2: Into<anyhow::Error> + Send + Sync,
    {
        match self {
            TransactionWorkflow::Import(cli_version) => {
                let intermediate_state: RT::CliStringRepr<U> = cli_version
                    .try_into()
                    .map_err(Into::<anyhow::Error>::into)?;
                let tx = intermediate_state
                    .try_into()
                    .map_err(Into::<anyhow::Error>::into)?;
                println!("Adding the following transaction to batch:");
                println!("{}", serde_json::to_string_pretty(&tx)?);
                wallet_state.unsent_transactions.push(tx);
            }
        }
        Ok(())
    }
}

// #[derive(clap::Parser)]
// /// Generate, sign, and send transactions
// pub enum TransactionWorkflow<T>
// where
//     T: clap::Subcommand + Send + Sync,
// {
//     /// Import a transaction  as a JSON string
//     #[clap(subcommand)]
//     Import(ImportTransaction<T>),
//     /// List the current batch of transactions
//     List,
//     // TODO: Add `send` and `generate_schema` subcommands/
//     // TODO: design and implement batch management (remove tx, drop batch, etc.)
// }

// impl TransactionWorkflow<T> {
//     /// Run the transaction workflow
//     pub fn run<E1, E2, C: sov_modules_api::Context, RT: CliWallet>(
//         self,
//         wallet_state: &mut WalletState<RT::Decodable, C>,
//         _app_dir: impl AsRef<Path>,
//     ) -> Result<(), anyhow::Error>
//     where
//         RT::Decodable: Serialize + DeserializeOwned,
//         RT::CliStringRepr: TryInto<RT::Decodable, Error = E1>,
//         T: TryInto<RT::CliStringRepr, Error = E2>,
//         E1: Into<anyhow::Error> + Send + Sync,
//         E2: Into<anyhow::Error> + Send + Sync,
//     {
//         match self {
//             TransactionWorkflow::Import(method) => {
//                 match method {
//                     ImportTransaction::FromFile(path_to_json) => {
//                         std::fs::read(&path_to_json.as_ref()).with_context(|| {
//                             format!("Could not open file at {}", path_to_json.as_ref())
//                         })?;
//                     }
//                     ImportTransaction::FromString(json) => json,
//                 };
//                 let tx = json.try_into().map_err(Into::into)?;
//                 println!("Adding the following transaction to batch:");
//                 println!("{}", serde_json::to_string_pretty(&tx)?);
//                 wallet_state.unsent_transactions.push(tx);
//             }
//             TransactionWorkflow::List => {
//                 println!("Current batch:");
//                 println!(
//                     "{}",
//                     serde_json::to_string_pretty(&wallet_state.unsent_transactions)?
//                 );
//             }
//         }

//         Ok(())
//     }
// }

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

// impl<T, Json, File, E1, E2> TryInto<T> for ImportTransaction<Json, File>
// where
//     Json: TryInto<T, Error = E1>,
//     File: TryInto<T, Error = E2>,
//     E1: Into<anyhow::Error> + Send + Sync,
//     E2: Into<anyhow::Error> + Send + Sync,
// {
//     type Error = anyhow::Error;

//     fn try_into(self) -> Result<T, Self::Error> {
//         match self {
//             ImportTransaction::FromFile(f) => f.try_into()?,
//             ImportTransaction::FromString(s) => s.try_into()?,
//         }
//     }
// }

// /// The optional arguments for the transaction workflow
// #[derive(Debug, Args)]
// pub struct OptionalArgs {
//     #[clap(short, long, global = true, default_value_t = false)]
//     send_transactions: bool,
// }
