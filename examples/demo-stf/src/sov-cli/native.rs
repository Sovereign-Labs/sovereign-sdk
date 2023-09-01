use std::path::PathBuf;

use demo_stf::runtime::{Runtime, RuntimeCall, RuntimeSubcommand};
use sov_cli::wallet_state::WalletState;
use sov_cli::workflows::keys::KeyWorkflow;
use sov_cli::workflows::rpc::RpcWorkflows;
use sov_cli::workflows::transactions::TransactionWorkflow;
use sov_cli::{clap, wallet_dir};
use sov_modules_api::clap::Parser;
use sov_modules_api::cli::{FileNameArg, JsonStringArg};
use sov_modules_api::default_context::DefaultContext;
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::mocks::MockDaSpec;

type Ctx = DefaultContext;

#[derive(clap::Subcommand)]
#[command(author, version, about, long_about = None)]
pub enum Workflows<Da: DaSpec> {
    #[clap(subcommand)]
    Transactions(
        TransactionWorkflow<
            RuntimeSubcommand<FileNameArg, DefaultContext, Da>,
            RuntimeSubcommand<JsonStringArg, DefaultContext, Da>,
        >,
    ),
    #[clap(subcommand)]
    Keys(KeyWorkflow<Ctx>),
    #[clap(subcommand)]
    Rpc(RpcWorkflows<Ctx>),
}

#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
pub struct App<Da: DaSpec> {
    #[clap(subcommand)]
    workflow: Workflows<Da>,
}

async fn run_workflow<Da: DaSpec>(
    app_dir: impl AsRef<std::path::Path>,
    wallet_state_path: PathBuf,
) -> Result<(), anyhow::Error> {
    let mut wallet_state: WalletState<RuntimeCall<Ctx, Da>, Ctx> =
        WalletState::load(&wallet_state_path)?;

    let invocation = App::<Da>::parse();

    match invocation.workflow {
        Workflows::Transactions(tx) => tx
            .run::<Runtime<DefaultContext, Da>, DefaultContext, JsonStringArg, _, _, _>(
                &mut wallet_state,
                app_dir,
            )?,
        Workflows::Keys(inner) => inner.run(&mut wallet_state, app_dir)?,
        Workflows::Rpc(inner) => {
            inner.run(&mut wallet_state, app_dir).await?;
        }
    }
    wallet_state.save(wallet_state_path)
}

pub async fn main() -> Result<(), anyhow::Error> {
    let app_dir = wallet_dir()?;
    std::fs::create_dir_all(app_dir.as_ref())?;
    let wallet_state_path = app_dir.as_ref().join("wallet_state.json");

    // TODO: Add params here
    run_workflow::<MockDaSpec>(app_dir, wallet_state_path).await
}
