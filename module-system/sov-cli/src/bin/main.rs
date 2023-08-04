use std::path::{Path, PathBuf};

use borsh::BorshSerialize;
use demo_stf::runtime::{CliTransactionParser, Runtime, RuntimeCall};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_cli::{clap, wallet_dir, KeyWorkflow, WalletState};
use sov_modules_api::clap::{Args, Parser, Subcommand};

type Ctx = sov_modules_api::default_context::DefaultContext;

#[derive(clap::Subcommand)]
#[command(author, version, about, long_about = None)]
pub enum Workflows {
    #[clap(subcommand)]
    Transactions(TransactionWorkflow<Runtime<Ctx>>),
    #[clap(subcommand)]
    Keys(KeyWorkflow<Ctx>),
    PrintBatch,
}

#[derive(clap::Parser)]
/// Generate, sign, and send transactions
pub enum TransactionWorkflow<RT: sov_modules_api::CliWallet>
where
    RT::CliStringRepr: clap::Subcommand,
{
    #[clap(subcommand)]
    Generate(RT::CliStringRepr),
    // PrintSchema,
    Import(TransactionSubcommand<ImportTransaction>),
    List,
    // Send(C::Address),
}

impl<RT: sov_modules_api::CliWallet> TransactionWorkflow<RT>
where
    RT::Decodable: Serialize + DeserializeOwned,
    RT::CliStringRepr: clap::Subcommand,
{
    pub fn run<C: sov_modules_api::Context>(
        self,
        wallet_state: &mut WalletState<RT::Decodable, C>,
        app_dir: impl AsRef<Path>,
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
                let TransactionSubcommand { args, inner } = subcommand;
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
    FromFile { path: PathBuf },
    /// Provide a JSON serialized transaction directly as input
    FromString { json: String },
}

#[derive(clap::Parser)]
pub struct TransactionSubcommand<S: Subcommand> {
    #[clap(flatten)]
    pub args: OptionalArgs,
    #[clap(subcommand)]
    pub inner: S,
}

#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
pub struct App {
    #[clap(subcommand)]
    workflow: Workflows,
}

#[derive(Debug, Args)]
pub struct OptionalArgs {
    #[clap(short, long, global = true, default_value_t = false)]
    send_transactions: bool,
}

fn main() -> Result<(), anyhow::Error> {
    let app_dir = wallet_dir()?;
    std::fs::create_dir_all(app_dir.as_ref())?;
    let wallet_state_path = app_dir.as_ref().join("wallet_state.json");
    let mut wallet_state: WalletState<RuntimeCall<Ctx>, Ctx> =
        WalletState::load(&wallet_state_path)?;

    let invocation = App::parse();

    match invocation.workflow {
        Workflows::Transactions(tx) => tx.run(&mut wallet_state, app_dir)?,
        Workflows::PrintBatch => {
            println!("Current batch:");
            println!(
                "{}",
                serde_json::to_string_pretty(&wallet_state.unsent_transactions)?
            );
        }
        Workflows::Keys(inner) => inner.run(&mut wallet_state, app_dir)?,
    }
    wallet_state.save(wallet_state_path)?;

    // if invocation.args.send_transactions {
    //     println!("Sending transactions!");
    // }

    Ok(())
}

pub fn save_txs(
    txs: Vec<RuntimeCall<Ctx>>,
    app_dir: impl AsRef<Path>,
) -> Result<(), anyhow::Error> {
    let txs_path = app_dir.as_ref().join("unsent_transactions.json");
    let txs = &txs.try_to_vec()?;
    std::fs::write(txs_path, txs)?;
    Ok(())
}
