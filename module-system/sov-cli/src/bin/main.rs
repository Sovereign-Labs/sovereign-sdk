use demo_stf::runtime::{Runtime, RuntimeCall, RuntimeSubcommand};
use sov_cli::wallet_state::WalletState;
use sov_cli::workflows::keys::KeyWorkflow;
use sov_cli::workflows::rpc::RpcWorkflows;
use sov_cli::workflows::transactions::{ImportTransaction, TransactionWorkflow};
use sov_cli::{clap, wallet_dir};
use sov_modules_api::clap::Parser;
use sov_modules_api::cli::{FileStringArg, JsonStringArg};
use sov_modules_api::default_context::DefaultContext;

type Ctx = sov_modules_api::default_context::DefaultContext;

#[derive(clap::Subcommand)]
#[command(author, version, about, long_about = None)]
pub enum Workflows {
    #[clap(subcommand)]
    Transactions(
        TransactionWorkflow<
            RuntimeSubcommand<FileStringArg, DefaultContext>,
            RuntimeSubcommand<JsonStringArg, DefaultContext>,
        >,
    ),
    // #[clap(subcommand)]
    // Transactions(TransactionWorkflow<RuntimeSubcommand<JsonStringArg, DefaultContext>>),
    #[clap(subcommand)]
    Keys(KeyWorkflow<Ctx>),
    #[clap(subcommand)]
    Rpc(RpcWorkflows<Ctx>),
}

#[derive(clap::Parser)]
#[command(author, version, about, long_about = None)]
pub struct App {
    #[clap(subcommand)]
    workflow: Workflows,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let app_dir = wallet_dir()?;
    std::fs::create_dir_all(app_dir.as_ref())?;
    let wallet_state_path = app_dir.as_ref().join("wallet_state.json");
    let mut wallet_state: WalletState<RuntimeCall<Ctx>, Ctx> =
        WalletState::load(&wallet_state_path)?;

    let invocation = App::parse();

    match invocation.workflow {
        Workflows::Transactions(tx) => tx
            .run::<Runtime<DefaultContext>, DefaultContext, JsonStringArg, _, _, _>(
                &mut wallet_state,
                app_dir,
            )?,
        Workflows::Keys(inner) => inner.run(&mut wallet_state, app_dir)?,
        Workflows::Rpc(inner) => {
            let _res = inner.run(&mut wallet_state, app_dir).await?;
        }
    }
    wallet_state.save(wallet_state_path)?;

    Ok(())
}
