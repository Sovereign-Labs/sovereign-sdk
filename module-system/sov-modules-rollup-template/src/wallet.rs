use async_trait::async_trait;
use borsh::BorshSerialize;
use sov_cli::wallet_state::WalletState;
use sov_cli::workflows::keys::KeyWorkflow;
use sov_cli::workflows::rpc::RpcWorkflows;
use sov_cli::workflows::transactions::TransactionWorkflow;
use sov_cli::{clap, wallet_dir};
use sov_modules_api::clap::Parser;
use sov_modules_api::cli::{CliFrontEnd, JsonStringArg};
use sov_modules_api::{CliWallet, Context, DispatchCall};

use crate::RollupTemplate;

#[derive(clap::Subcommand)]
#[command(author, version, about, long_about = None)]
enum Workflows<File: clap::Subcommand, Json: clap::Subcommand, C: Context> {
    #[clap(subcommand)]
    Transactions(TransactionWorkflow<File, Json>),
    #[clap(subcommand)]
    Keys(KeyWorkflow<C>),
    #[clap(subcommand)]
    Rpc(RpcWorkflows<C>),
}

#[derive(clap::Parser)]
#[command(author, version, about = None, long_about = None)]
struct CliApp<File: clap::Subcommand, Json: clap::Subcommand, C: Context> {
    #[clap(subcommand)]
    workflow: Workflows<File, Json, C>,
}

/// Generic wallet for any type that implements RollupTemplate.
#[async_trait]
pub trait WalletTemplate: RollupTemplate
where
    // The types here a quite complicated but they are never exposed to the user
    // as the WalletTemplate is implemented for any types that implements RollupTemplate.
    <Self as RollupTemplate>::NativeContext:
        serde::Serialize + serde::de::DeserializeOwned + Send + Sync,

    <Self as RollupTemplate>::NativeRuntime: CliWallet,

    <Self as RollupTemplate>::DaSpec: serde::Serialize + serde::de::DeserializeOwned,

    <<Self as RollupTemplate>::NativeRuntime as DispatchCall>::Decodable:
        serde::Serialize + serde::de::DeserializeOwned + BorshSerialize + Send + Sync,

    <<Self as RollupTemplate>::NativeRuntime as CliWallet>::CliStringRepr<JsonStringArg>: TryInto<
        <<Self as RollupTemplate>::NativeRuntime as DispatchCall>::Decodable,
        Error = serde_json::Error,
    >,
{
    /// Generates wallet cli for the runtime.
    async fn run_wallet<File: clap::Subcommand, Json: clap::Subcommand>(
    ) -> Result<(), anyhow::Error>
    where
        File: CliFrontEnd<<Self as RollupTemplate>::NativeRuntime> + Send + Sync,
        Json: CliFrontEnd<<Self as RollupTemplate>::NativeRuntime> + Send + Sync,

        File: TryInto<
            <<Self as RollupTemplate>::NativeRuntime as CliWallet>::CliStringRepr<JsonStringArg>,
            Error = std::io::Error,
        >,
        Json: TryInto<
            <<Self as RollupTemplate>::NativeRuntime as CliWallet>::CliStringRepr<JsonStringArg>,
            Error = std::convert::Infallible,
        >,
    {
        let app_dir = wallet_dir()?;

        std::fs::create_dir_all(app_dir.as_ref())?;
        let wallet_state_path = app_dir.as_ref().join("wallet_state.json");

        let mut wallet_state: WalletState<
            <<Self as RollupTemplate>::NativeRuntime as DispatchCall>::Decodable,
            <Self as RollupTemplate>::NativeContext,
        > = WalletState::load(&wallet_state_path)?;

        let invocation = CliApp::<File, Json, <Self as RollupTemplate>::NativeContext>::parse();

        match invocation.workflow {
            Workflows::Transactions(tx) => tx
                .run::<<Self as RollupTemplate>::NativeRuntime, <Self as RollupTemplate>::NativeContext, JsonStringArg, _, _, _>(
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
}
