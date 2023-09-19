//! Query the current state of the rollup and send transactions

use std::path::Path;

use anyhow::Context;
use borsh::BorshSerialize;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::HttpClientBuilder;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_accounts::AccountsRpcClient;
use sov_bank::{BalanceResponse, BankRpcClient};
use sov_modules_api::clap;
use sov_modules_api::transaction::Transaction;

use crate::wallet_state::{AddressEntry, KeyIdentifier, WalletState};
use crate::workflows::keys::load_key;
const NO_ACCOUNTS_FOUND: &str =
    "No accounts found. You can generate one with the `keys generate` subcommand";
const BAD_RPC_URL: &str = "Unable to connect to provided rpc. You can change to a different rpc url with the `rpc set-url` subcommand ";

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
    /// Sign all transactions from the current batch and submit them to the rollup.
    /// Nonces will be set automatically.
    SubmitBatch {
        /// (Optional) The account to sign transactions for this batch (default: the active account)
        #[clap(subcommand)]
        account: Option<KeyIdentifier<C>>,
        /// (Optional) The nonce to use for the first transaction in the batch (default: the current nonce for the account). Any other transactions will
        /// be signed with sequential nonces starting from this value.
        nonce_override: Option<u64>,
    },
}

impl<C: sov_modules_api::Context> RpcWorkflows<C> {
    fn resolve_account<'wallet, Tx: BorshSerialize>(
        &self,
        wallet_state: &'wallet mut WalletState<Tx, C>,
    ) -> Result<&'wallet AddressEntry<C>, anyhow::Error> {
        let account_id = match self {
            RpcWorkflows::SetUrl { .. } => None,
            RpcWorkflows::GetNonce { account } => account.as_ref(),
            RpcWorkflows::GetBalance { account, .. } => account.as_ref(),
            RpcWorkflows::SubmitBatch { account, .. } => account.as_ref(),
        };

        let account = if let Some(id) = account_id {
            let addr = wallet_state.addresses.get_address(id);

            addr.ok_or_else(|| anyhow::format_err!("No account found matching identifier: {}", id))?
        } else {
            wallet_state
                .addresses
                .default_address()
                .ok_or_else(|| anyhow::format_err!(NO_ACCOUNTS_FOUND))?
        };
        Ok(account)
    }
}

impl<C: sov_modules_api::Context + Serialize + DeserializeOwned + Send + Sync> RpcWorkflows<C> {
    /// Run the rpc workflow
    pub async fn run<Tx: BorshSerialize>(
        &self,
        wallet_state: &mut WalletState<Tx, C>,
        _app_dir: impl AsRef<Path>,
    ) -> Result<(), anyhow::Error> {
        // If the user is just setting the RPC url, we can skip the usual setup
        if let RpcWorkflows::SetUrl { rpc_url } = self {
            let _client = HttpClientBuilder::default()
                .build(rpc_url)
                .context("Invalid rpc url: ")?;
            wallet_state.rpc_url = Some(rpc_url.clone());
            println!("Set rpc url to {}", rpc_url);
            return Ok(());
        }

        // Otherwise, we need to initialize an  RPC and resolve the active account
        let rpc_url = wallet_state
            .rpc_url
            .as_ref()
            .ok_or(anyhow::format_err!(
                "No rpc url set. Use the `rpc set-url` subcommand to set one"
            ))?
            .clone();
        let client = HttpClientBuilder::default().build(rpc_url)?;
        let account = self.resolve_account(wallet_state)?;

        // Finally, run the workflow
        match self {
            RpcWorkflows::SetUrl { .. } => {
                unreachable!("This case was handled above")
            }
            RpcWorkflows::GetNonce { .. } => {
                let nonce = get_nonce_for_account(&client, account).await?;
                println!("Nonce for account {} is {}", account.address, nonce);
            }
            RpcWorkflows::GetBalance {
                account: _,
                token_address,
            } => {
                let BalanceResponse { amount } = BankRpcClient::<C>::balance_of(
                    &client,
                    account.address.clone(),
                    token_address.clone(),
                )
                .await
                .context(BAD_RPC_URL)?;

                println!(
                    "Balance for account {} is {}",
                    account.address,
                    amount.unwrap_or_default()
                );
            }
            RpcWorkflows::SubmitBatch { nonce_override, .. } => {
                let private_key = load_key::<C>(&account.location)?;

                let nonce = match nonce_override {
                    Some(nonce) => *nonce,
                    None => get_nonce_for_account(&client, account).await?,
                };
                let txs = std::mem::take(&mut wallet_state.unsent_transactions)
                    .into_iter()
                    .enumerate()
                    .map(|(offset, tx)| {
                        Transaction::<C>::new_signed_tx(
                            &private_key,
                            tx.try_to_vec().unwrap(),
                            nonce + offset as u64,
                        )
                        .try_to_vec()
                        .unwrap()
                    })
                    .collect::<Vec<_>>();

                let response: String = client
                    .request("sequencer_publishBatch", txs)
                    .await
                    .context("Unable to publish batch")?;

                // Print the result
                println!(
                    "Your batch was submitted to the sequencer for publication. Response: {:?}",
                    response
                );
            }
        }
        Ok(())
    }
}

async fn get_nonce_for_account<C: sov_modules_api::Context + Send + Sync + Serialize>(
    client: &(impl ClientT + Send + Sync),
    account: &AddressEntry<C>,
) -> Result<u64, anyhow::Error> {
    Ok(match AccountsRpcClient::<C>::get_account(
        client,
        account.pub_key.clone(),
    )
    .await
    .context(
        "Unable to connect to provided rpc. You can change to a different rpc url with the `rpc set-url` subcommand ",
    )? {
        sov_accounts::Response::AccountExists { addr: _, nonce } => nonce,
        _ => 0,
    })
}
