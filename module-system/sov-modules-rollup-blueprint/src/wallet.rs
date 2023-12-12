use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_cli::wallet_state::WalletState;
use sov_cli::workflows::keys::KeyWorkflow;
use sov_cli::workflows::rpc::RpcWorkflows;
use sov_cli::workflows::transactions::TransactionWorkflow;
use sov_cli::{clap, wallet_dir};
use sov_modules_api::clap::Parser;
use sov_modules_api::cli::{CliFrontEnd, CliTxImportArg, JsonStringArg};
use sov_modules_api::{CliWallet, Context, DispatchCall};

use crate::RollupBlueprint;

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

/// Generic wallet for any type that implements RollupBlueprint.
#[async_trait]
pub trait WalletBlueprint: RollupBlueprint
where
    // The types here a quite complicated but they are never exposed to the user
    // as the WalletTemplate is implemented for any types that implements RollupBlueprint.
    <Self as RollupBlueprint>::NativeContext:
        serde::Serialize + serde::de::DeserializeOwned + Send + Sync,

    <Self as RollupBlueprint>::NativeRuntime: CliWallet,

    <Self as RollupBlueprint>::DaSpec: serde::Serialize + serde::de::DeserializeOwned,

    <<Self as RollupBlueprint>::NativeRuntime as DispatchCall>::Decodable:
        serde::Serialize + serde::de::DeserializeOwned + BorshSerialize + Send + Sync,

    <<Self as RollupBlueprint>::NativeRuntime as CliWallet>::CliStringRepr<JsonStringArg>: TryInto<
        <<Self as RollupBlueprint>::NativeRuntime as DispatchCall>::Decodable,
        Error = serde_json::Error,
    >,
{
    /// Generates wallet cli for the runtime.
    async fn run_wallet<File: clap::Subcommand, Json: clap::Subcommand>(
    ) -> Result<(), anyhow::Error>
    where
        <<Self as RollupBlueprint>::NativeRuntime as DispatchCall>::Decodable:
            BorshSerialize + BorshDeserialize + Serialize + DeserializeOwned,
        File: CliFrontEnd<<Self as RollupBlueprint>::NativeRuntime> + CliTxImportArg + Send + Sync,
        Json: CliFrontEnd<<Self as RollupBlueprint>::NativeRuntime> + CliTxImportArg + Send + Sync,

        File: TryInto<
            <<Self as RollupBlueprint>::NativeRuntime as CliWallet>::CliStringRepr<JsonStringArg>,
            Error = std::io::Error,
        >,
        Json: TryInto<
            <<Self as RollupBlueprint>::NativeRuntime as CliWallet>::CliStringRepr<JsonStringArg>,
            Error = std::convert::Infallible,
        >,
    {
        let app_dir = wallet_dir()?;

        std::fs::create_dir_all(app_dir.as_ref())?;
        let wallet_state_path = app_dir.as_ref().join("wallet_state.json");

        let mut wallet_state: WalletState<
            <<Self as RollupBlueprint>::NativeRuntime as DispatchCall>::Decodable,
            <Self as RollupBlueprint>::NativeContext,
        > = WalletState::load(&wallet_state_path)?;

        let invocation = CliApp::<File, Json, <Self as RollupBlueprint>::NativeContext>::parse();

        match invocation.workflow {
            Workflows::Transactions(tx) => tx
                .run::<<Self as RollupBlueprint>::NativeRuntime, <Self as RollupBlueprint>::NativeContext, JsonStringArg, _, _, _>(
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
