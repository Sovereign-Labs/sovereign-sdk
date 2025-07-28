use std::str::FromStr;

use async_trait::async_trait;
use borsh::{BorshDeserialize, BorshSerialize};
use sov_cli::wallet_state::WalletState;
use sov_cli::workflows::keys::KeyWorkflow;
use sov_cli::workflows::node::NodeWorkflows;
use sov_cli::workflows::transactions::TransactionWorkflow;
use sov_cli::{clap, wallet_dir};
use sov_modules_api::clap::Parser;
use sov_modules_api::cli::{CliFrontEnd, CliTxImportArg, JsonStringArg};
use sov_modules_api::execution_mode::ExecutionMode;
use sov_modules_api::{CliWallet, DispatchCall, Spec};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter, Layer};

use crate::{FullNodeBlueprint, RollupBlueprint};

#[derive(clap::Subcommand)]
#[command(author, version, about, long_about = None)]
enum Workflows<File: clap::Subcommand, Json: clap::Subcommand, S: Spec> {
    #[clap(subcommand)]
    Transactions(TransactionWorkflow<File, Json>),
    #[clap(subcommand)]
    Keys(KeyWorkflow<S>),
    #[clap(subcommand)]
    Node(NodeWorkflows<S>),
}

#[derive(clap::Parser)]
#[command(author, version, about = None, long_about = None)]
struct CliApp<File: clap::Subcommand, Json: clap::Subcommand, S: Spec> {
    #[clap(subcommand)]
    workflow: Workflows<File, Json, S>,
}

/// Generic wallet for any type that implements FullNodeBlueprint.
#[async_trait]
pub trait WalletBlueprint<M: ExecutionMode>: FullNodeBlueprint<M>
where
    // The types here are quite complicated, but they are never exposed to the user
    // as the WalletTemplate is implemented for any types that implements FullNodeBlueprint.
    Self::Spec: serde::Serialize + serde::de::DeserializeOwned + Send + Sync,

    <Self as RollupBlueprint<M>>::Runtime: CliWallet,

    <<Self as RollupBlueprint<M>>::Spec as Spec>::Da:
        serde::Serialize + serde::de::DeserializeOwned,

    <<Self as RollupBlueprint<M>>::Runtime as DispatchCall>::Decodable:
        serde::Serialize + serde::de::DeserializeOwned + BorshSerialize + Send + Sync,

    <<Self as RollupBlueprint<M>>::Runtime as CliWallet>::CliStringRepr<JsonStringArg>: TryInto<
        <<Self as RollupBlueprint<M>>::Runtime as DispatchCall>::Decodable,
        Error = serde_json::Error,
    >,
{
    /// Generates wallet cli for the runtime.
    async fn run_wallet<File: clap::Subcommand, Json: clap::Subcommand>() -> anyhow::Result<()>
    where
        <<Self as RollupBlueprint<M>>::Runtime as DispatchCall>::Decodable:
            BorshSerialize + BorshDeserialize + serde::Serialize + serde::de::DeserializeOwned,
        File: CliFrontEnd<<Self as RollupBlueprint<M>>::Runtime> + CliTxImportArg + Send + Sync,
        Json: CliFrontEnd<<Self as RollupBlueprint<M>>::Runtime> + CliTxImportArg + Send + Sync,

        File: TryInto<
            <<Self as RollupBlueprint<M>>::Runtime as CliWallet>::CliStringRepr<JsonStringArg>,
            Error = std::io::Error,
        >,
        Json: TryInto<
            <<Self as RollupBlueprint<M>>::Runtime as CliWallet>::CliStringRepr<JsonStringArg>,
            Error = std::convert::Infallible,
        >,
    {
        let rust_log = std::env::var("RUST_LOG").unwrap_or("info".to_string());
        tracing_subscriber::registry()
            .with(fmt::layer().with_filter(EnvFilter::from_str(&rust_log)?))
            .init();
        let app_dir = wallet_dir()?;

        std::fs::create_dir_all(app_dir.as_ref())?;
        let wallet_state_path = app_dir.as_ref().join("wallet_state.json");

        let mut wallet_state: WalletState<<Self as RollupBlueprint<M>>::Runtime, Self::Spec> =
            WalletState::load(&wallet_state_path)?;

        let invocation = CliApp::<File, Json, Self::Spec>::parse();

        match invocation.workflow {
            Workflows::Transactions(tx) => tx
                .run::<<Self as RollupBlueprint<M>>::Runtime, Self::Spec, JsonStringArg, _, _, _>(
                    &mut wallet_state,
                    app_dir,
                    std::io::stdout(),
                )?,
            Workflows::Keys(inner) => inner.run(&mut wallet_state, app_dir)?,
            Workflows::Node(inner) => {
                inner.run(&mut wallet_state, app_dir).await?;
            }
        }

        wallet_state.save(wallet_state_path)
    }
}
