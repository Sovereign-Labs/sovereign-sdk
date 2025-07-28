//! Workflows for interacting with Rollup node API
use std::path::Path;

use anyhow::Context;
use futures::StreamExt;
use reqwest::Url;
use serde::de::DeserializeOwned;
use serde::Serialize;
use sov_bank::TokenId;
use sov_modules_api::digest::Digest;
use sov_modules_api::prelude::tokio;
use sov_modules_api::{clap, DispatchCall};
use sov_rollup_interface::zk::CryptoSpec;
use sov_rollup_interface::TxHash;

use crate::node_client::NodeClient;
use crate::wallet_state::{KeyIdentifier, WalletState};
use crate::workflows::keys::load_key;

/// Query the current state of the rollup and send transactions.
#[derive(clap::Subcommand)]
pub enum NodeWorkflows<S: sov_modules_api::Spec> {
    /// Set URL to the rollup node.
    SetUrl {
        /// A URL to the REST API root endpoint http://localhost:12346
        url: String,
    },
    /// Query rollup node for the nonce of the provided account. If no account is provided, the active account is used.
    GetNonce {
        /// (Optional) The account to query the nonce for (default: the active account)
        #[clap(subcommand)]
        account: Option<KeyIdentifier<S>>,
    },
    /// Query the address of token by name and owner
    FindTokenId {
        /// The name of the token to query for
        token_name: String,
        /// The token's decimals
        token_decimals: Option<u8>,
        /// The deployer of the token.
        /// In the case of genesis token, it can be looked up in genesis config JSON.
        /// Check the server logs if it does not match.
        deployer_address: S::Address,
    },
    /// Query the rollup nod for the token balance of an account
    GetBalance {
        /// (Optional) The account to query the balance of (default: the active account)
        #[clap(subcommand)]
        account: Option<KeyIdentifier<S>>,
        /// The ID of the token to query for
        token_id: TokenId,
    },
    /// Sign all transactions from the current batch and submit them to the rollup.
    /// Nonces will be set automatically.
    SubmitBatch {
        /// (Optional) The account to sign transactions for this batch (default: the active account)
        #[clap(subcommand)]
        account: Option<KeyIdentifier<S>>,
        /// (Optional) Waits for a given batch to be processed by the rollup node.
        #[arg(short, long)]
        wait_for_processing: bool,
        /// (Optional) The nonce to use for the first transaction in the batch (default: the current nonce for the account). Any other transactions will
        /// be signed with sequential nonces starting from this value.
        nonce_override: Option<u64>,
    },
    /// Waits for the aggregated proof to become available.
    /// This waits for the next aggregated proof after the command is invoked.
    WaitForAggregatedProof {
        /// The number of seconds to wait before timing out.
        timeout_secs: Option<u64>,
    },
}

impl<S: sov_modules_api::Spec + Serialize + DeserializeOwned> NodeWorkflows<S> {
    /// Runs API workflow.
    pub async fn run<Tx>(
        &self,
        wallet_state: &mut WalletState<Tx, S>,
        _app_dir: impl AsRef<Path>,
    ) -> anyhow::Result<()>
    where
        Tx: DispatchCall,
        Tx::Decodable: Serialize + DeserializeOwned,
    {
        tracing::info!("Executing node workflow");
        if let Self::SetUrl { url } = self {
            Url::parse(url).map_err(|e| anyhow::anyhow!("Failed to parse API URL: {:?}", e))?;
            let prev_url = wallet_state.rest_api_url.clone();
            wallet_state.rest_api_url = Some(url.clone());
            tracing::info!(?prev_url, url, "Set REST API URL");
            return Ok(());
        }

        let url = wallet_state
            .rest_api_url
            .as_ref()
            .ok_or(anyhow::format_err!(
                "No REST API URL set. Use the `api set-url` subcommand to set one"
            ))?
            .clone();

        let api_client = NodeClient::new(&url).await?;

        match self {
            NodeWorkflows::SetUrl { .. } => {
                unreachable!("This case was handled above")
            }
            NodeWorkflows::GetNonce { account } => {
                let account = wallet_state.resolve_account(account.as_ref())?;
                let nonce = api_client
                    .get_nonce_for_public_key::<S>(&account.pub_key)
                    .await?;
                tracing::info!(
                    account = %account.address,
                    nonce,
                    "Nonce for account received"
                );
            }
            NodeWorkflows::FindTokenId {
                token_name,
                token_decimals,
                deployer_address,
            } => {
                let token_id = api_client
                    .get_token_id::<S>(token_name, *token_decimals, deployer_address)
                    .await?;
                tracing::info!(%token_name, %token_id, "Id of token is received");
            }
            NodeWorkflows::GetBalance { account, token_id } => {
                let account = wallet_state.resolve_account(account.as_ref())?;
                let balance = api_client
                    .get_balance::<S>(&account.address, token_id, None)
                    .await?;
                tracing::info!(
                    "Balance of token {} for account {} is {}",
                    token_id,
                    account.address,
                    balance
                );

                return Ok(());
            }
            NodeWorkflows::SubmitBatch {
                account,
                wait_for_processing,
                nonce_override,
            } => {
                let account = wallet_state.resolve_account(account.as_ref())?;
                let private_key = load_key::<S>(&account.location).with_context(|| {
                    format!("Unable to load key {}", account.location.display())
                })?;

                let nonce = match nonce_override {
                    Some(nonce) => *nonce,
                    None => {
                        api_client
                            .get_nonce_for_public_key::<S>(&account.pub_key)
                            .await?
                    }
                };

                let txs = wallet_state.take_signed_transactions(&private_key, nonce);

                for (i, tx) in txs.iter().enumerate() {
                    let tx_hash =
                        TxHash::new(<S::CryptoSpec as CryptoSpec>::Hasher::digest(tx).into());
                    tracing::info!(index = i, %tx_hash, "Submitting tx");
                }

                api_client
                    .send_transactions_to_sequencer(txs, *wait_for_processing)
                    .await?;
            }
            NodeWorkflows::WaitForAggregatedProof { timeout_secs } => {
                let timeout = std::time::Duration::from_secs(timeout_secs.unwrap_or(120));
                tracing::info!(?timeout, "Subscribing for aggregated proofs");
                let mut aggregated_proofs = api_client.client.subscribe_aggregated_proof().await?;
                let aggregated_proof = tokio::time::timeout(timeout, aggregated_proofs.next())
                    .await?
                    .transpose()?
                    .ok_or_else(|| anyhow::anyhow!("Missing aggregated proof in response body"))?;
                tracing::info!(?aggregated_proof, "Aggregated proof received");
            }
        }

        Ok(())
    }
}
