//! Workflows for transaction management

use std::path::Path;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_api::clap::{self, Subcommand};
use sov_modules_api::cli::{CliFrontEnd, CliTxImportArg};
use sov_modules_api::transaction::UnsignedTransaction;
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
        File: CliFrontEnd<RT> + CliTxImportArg,
        Json: CliFrontEnd<RT> + CliTxImportArg,
        File: TryInto<RT::CliStringRepr<V>, Error = E1>,
        Json: TryInto<RT::CliStringRepr<V>, Error = E2>,
        RT::CliStringRepr<V>: TryInto<RT::Decodable, Error = E3>,
        RT::Decodable: BorshSerialize + BorshDeserialize + Serialize + DeserializeOwned,
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
    pub fn run<RT: CliWallet, C: sov_modules_api::Context, U, E1, E2, E3>(
        self,
        wallet_state: &mut WalletState<RT::Decodable, C>,
    ) -> Result<(), anyhow::Error>
    where
        Json: CliFrontEnd<RT> + CliTxImportArg,
        File: CliFrontEnd<RT> + CliTxImportArg,
        Json: TryInto<RT::CliStringRepr<U>, Error = E1>,
        File: TryInto<RT::CliStringRepr<U>, Error = E2>,
        RT::CliStringRepr<U>: TryInto<RT::Decodable, Error = E3>,
        RT::Decodable: BorshSerialize + BorshDeserialize + Serialize + DeserializeOwned,
        E1: Into<anyhow::Error> + Send + Sync,
        E2: Into<anyhow::Error> + Send + Sync,
        E3: Into<anyhow::Error> + Send + Sync,
    {
        let chain_id;
        let gas_tip;
        let gas_limit;

        let intermediate_repr: RT::CliStringRepr<U> = match self {
            ImportTransaction::FromFile(file) => {
                chain_id = file.chain_id();
                gas_tip = file.gas_tip();
                gas_limit = file.gas_limit();
                file.try_into().map_err(Into::<anyhow::Error>::into)?
            }
            ImportTransaction::FromString(json) => {
                chain_id = json.chain_id();
                gas_tip = json.gas_tip();
                gas_limit = json.gas_limit();
                json.try_into().map_err(Into::<anyhow::Error>::into)?
            }
        };

        let tx: RT::Decodable = intermediate_repr
            .try_into()
            .map_err(Into::<anyhow::Error>::into)?;

        let tx = UnsignedTransaction::new(tx, chain_id, gas_tip, gas_limit);

        println!("Adding the following transaction to batch:");
        println!("{}", serde_json::to_string_pretty(&tx)?);

        wallet_state.unsent_transactions.push(tx);

        Ok(())
    }
}
