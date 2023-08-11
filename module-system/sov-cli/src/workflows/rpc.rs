//! Query the current state of the rollup and send transactions

use std::path::Path;

use anyhow::Context;
use demo_stf::runtime::query::accounts::{self, AccountsRpcClient};
use demo_stf::runtime::query::bank::BankRpcClient;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_modules_api::clap;

use crate::wallet_state::{KeyIdentifier, WalletState};

/// Query the current state of the rollup and send transactions
#[derive(clap::Subcommand)]
pub enum RpcWorkflows<C: sov_modules_api::Context> {
    /// Set the url of the rpc server to use
    SetUrl {
        /// A url like http://localhost:8545
        rpc_url: String,
    },
    /// Query the rpc server for the nonce of the provided account. If no account is provided, the active account is used
    GetNonce {
        /// (Optional) The account to query the nonce for (default: the active account)
        #[clap(subcommand)]
        account: Option<KeyIdentifier<C>>,
    },
    /// Query the rpc server for the token balance of an account
    GetBalance {
        /// (Optional) The account to query the balance of (default: the active account)
        #[clap(subcommand)]
        account: Option<KeyIdentifier<C>>,
        /// The address of the token to query for
        token_address: C::Address,
    },
}

impl<C: sov_modules_api::Context + Serialize + DeserializeOwned + Send + Sync> RpcWorkflows<C> {
    /// Run the rpc workflow
    pub async fn run<Tx>(
        &self,
        wallet_state: &mut WalletState<Tx, C>,
        app_dir: impl AsRef<Path>,
    ) -> Result<(), anyhow::Error> {
        match self {
            RpcWorkflows::SetUrl { rpc_url } => {
                let client = HttpClientBuilder::default()
                    .build(rpc_url)
                    .context("Invalid rpc url: ")?;
                wallet_state.rpc_url = Some(rpc_url.clone());
                println!("Set rpc url to {}", rpc_url);
            }
            RpcWorkflows::GetNonce { account } => {
                let account = if let Some(id) = account {
                    let addr = wallet_state.addresses.get_address(id);
                    let addr = addr.ok_or_else(|| {
                        anyhow::format_err!("No account found matching identifier: {}", id)
                    })?;
                    addr
                } else {
                    wallet_state.addresses.default_address().ok_or_else(|| {
						anyhow::format_err!("No accounts found. You can generate one with the `keys generate` subcommand")
					})?
                };
                let rpc_url = wallet_state.rpc_url.as_ref().ok_or(anyhow::format_err!(
                    "No rpc url set. Use the `rpc set-url` subcommand to set one"
                ))?;
                let client = HttpClientBuilder::default().build(rpc_url)?;
                let nonce = match AccountsRpcClient::<C>::get_account(
                    &client,
                    account.pub_key.clone(),
                )
                .await
                .context(
                    "Unable to connect to provided rpc. You can change to a different rpc url with the `rpc set-url` subcommand ",
                )? {
                    accounts::Response::AccountExists { addr: _, nonce } => nonce,
                    _ => 0,
                };
                println!("Nonce for account {} is {}", account.address, nonce);
            }
            RpcWorkflows::GetBalance {
                account,
                token_address,
            } => {
                todo!()
                // let account = account
                //     .as_ref()
                //     .map(|id| wallet_state.get_address(id))
                //     .transpose()?
                //     .unwrap_or_else(|| wallet_state.get_default_address().unwrap().address);
                // let balance = wallet_state.get_balance(account, *token_address)?;
                // println!("Balance for account {} is {}", account, balance);
            }
        }
        Ok(())
    }
}
