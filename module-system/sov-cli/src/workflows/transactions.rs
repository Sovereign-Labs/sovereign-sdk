//! Workflows for transaction management

use std::path::Path;

use serde::Serialize;
use sov_modules_api::clap::{self, Subcommand};
use sov_modules_api::cli::CliFrontEnd;
use sov_modules_api::CliWallet;

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
    pub fn run<RT: CliWallet, C: sov_modules_api::Context, V, E1, E2, E3>(
        self,
        wallet_state: &mut WalletState<RT::Decodable, C>,
        _app_dir: impl AsRef<Path>,
    ) -> Result<(), anyhow::Error>
    where
        File: CliFrontEnd<RT>,
        Json: CliFrontEnd<RT>,
        File: TryInto<RT::CliStringRepr<V>, Error = E1>,
        Json: TryInto<RT::CliStringRepr<V>, Error = E2>,
        RT::CliStringRepr<V>: TryInto<RT::Decodable, Error = E3>,
        RT::Decodable: Serialize,
        E1: Into<anyhow::Error> + Send + Sync,
        E2: Into<anyhow::Error> + Send + Sync,
        E3: Into<anyhow::Error> + Send + Sync,
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

    // /// Run the transaction workflow
    // pub fn run<RT: CliWallet, C: sov_modules_api::Context, U, E1, E2>(
    //     self,
    //     wallet_state: &mut WalletState<RT::Decodable, C>,
    //     _app_dir: impl AsRef<Path>,
    // ) -> Result<(), anyhow::Error>
    // where
    //     T: CliFrontEnd<RT>,
    //     T: TryInto<RT::CliStringRepr<U>, Error = E1>,
    //     RT::CliStringRepr<U>: TryInto<RT::Decodable, Error = E2>,
    //     RT::Decodable: Serialize,
    //     E1: Into<anyhow::Error> + Send + Sync,
    //     E2: Into<anyhow::Error> + Send + Sync,
    // {
    //     match self {
    //         TransactionWorkflow::Import(cli_version) => {
    //             let intermediate_state: RT::CliStringRepr<U> = cli_version
    //                 .try_into()
    //                 .map_err(Into::<anyhow::Error>::into)?;
    //             let tx = intermediate_state
    //                 .try_into()
    //                 .map_err(Into::<anyhow::Error>::into)?;
    //             println!("Adding the following transaction to batch:");
    //             println!("{}", serde_json::to_string_pretty(&tx)?);
    //             wallet_state.unsent_transactions.push(tx);
    //         }
    //     }
    //     Ok(())
    // }
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

impl<Json, File> ImportTransaction<Json, File>
where
    Json: Subcommand,
    File: Subcommand,
{
    /// Parse from a file or a json string
    pub fn run<RT: CliWallet, C: sov_modules_api::Context, U, E1, E2, E3>(
        self,
        wallet_state: &mut WalletState<RT::Decodable, C>,
    ) -> Result<(), anyhow::Error>
    where
        Json: CliFrontEnd<RT>,
        File: CliFrontEnd<RT>,
        Json: TryInto<RT::CliStringRepr<U>, Error = E1>,
        File: TryInto<RT::CliStringRepr<U>, Error = E2>,
        RT::CliStringRepr<U>: TryInto<RT::Decodable, Error = E3>,
        RT::Decodable: Serialize,
        E1: Into<anyhow::Error> + Send + Sync,
        E2: Into<anyhow::Error> + Send + Sync,
        E3: Into<anyhow::Error> + Send + Sync,
    {
        let intermediate_repr: RT::CliStringRepr<U> = match self {
            ImportTransaction::FromFile(file) => {
                file.try_into().map_err(Into::<anyhow::Error>::into)?
            }
            ImportTransaction::FromString(json) => {
                json.try_into().map_err(Into::<anyhow::Error>::into)?
            }
        };

        let tx = intermediate_repr
            .try_into()
            .map_err(Into::<anyhow::Error>::into)?;
        println!("Adding the following transaction to batch:");
        println!("{}", serde_json::to_string_pretty(&tx)?);
        wallet_state.unsent_transactions.push(tx);
        Ok(())
    }
}

// /// The optional arguments for the transaction workflow
// #[derive(Debug, Args)]
// pub struct OptionalArgs {
//     #[clap(short, long, global = true, default_value_t = false)]
//     send_transactions: bool,
// }
