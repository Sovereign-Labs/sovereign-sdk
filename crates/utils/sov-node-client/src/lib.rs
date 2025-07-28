//! Contains a simple client to interact with sovereign rollup node

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::Context;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use futures::StreamExt;
use sov_api_spec::types;
use sov_api_spec::types::AcceptTxBody;
use sov_bank::utils::TokenHolder;
use sov_bank::{Amount, Coins, TokenId};
use sov_modules_api::prelude::tracing;
use sov_rollup_interface::crypto::{CredentialId, PublicKey};
use sov_rollup_interface::da::DaSpec;
use sov_rollup_interface::zk::CryptoSpec;
use sov_sequencer_registry::KnownSequencer;

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct NonceResponse {
    key: CredentialId,
    value: u64,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct TokenIdResponse {
    token_id: TokenId,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound = "S::Address: serde::Serialize + serde::de::DeserializeOwned")]
struct AdminsResponse<S: sov_modules_api::Spec> {
    admins: Vec<TokenHolder<S>>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound = "S::Address: serde::Serialize + serde::de::DeserializeOwned")]
struct KnownSequencerResponse<S: sov_modules_api::Spec> {
    key: <S::Da as DaSpec>::Address,
    value: KnownSequencer<S>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(bound = "S::Address: serde::Serialize + serde::de::DeserializeOwned")]
pub struct AllowedSequencer<S: sov_modules_api::Spec> {
    pub address: S::Address,
    pub balance: Amount,
}

/// NodeClient is a collection of helper methods that can interact with rollup node via REST API.
#[derive(Debug, Clone)]
pub struct NodeClient {
    /// Base URL where runtime, sequencer and ledger routers are mounted.
    pub base_url: String,
    /// Client that used to communicate with Runtime REST API.
    http_client: reqwest::Client,
    /// A [`sov_api_spec::Client`] for communication with the full node endpoints.
    pub client: sov_api_spec::Client,
}

impl NodeClient {
    /// Construct a new NodeClient without verifying that the target url is available and supports
    /// the required functionality.
    pub fn new_unchecked(api_url: &str) -> Self {
        let base_url = api_url.to_string();
        let http_client = reqwest::Client::new();
        let client = sov_api_spec::Client::new(api_url);

        NodeClient {
            base_url,
            http_client,
            client,
        }
    }

    /// Constructor. Implies base url for rollup node.
    pub async fn new(api_url: &str) -> anyhow::Result<Self> {
        let client = NodeClient::new_unchecked(api_url);
        if !check_if_rollup_has_standard_modules(&client.http_client, &client.base_url).await? {
            anyhow::bail!("Rollup does not have standard modules with standard names. Not all functions of sov-cli are available");
        }

        Ok(client)
    }

    /// Simplified constructor for testing.
    pub async fn new_at_localhost(port: u16) -> anyhow::Result<Self> {
        let api_url = format!("http://127.0.0.1:{port}");
        Self::new(&api_url).await
    }

    /// Simplified constructor for testing.
    pub fn new_at_localhost_unchecked(port: u16) -> Self {
        let api_url = format!("http://127.0.0.1:{port}");
        Self::new_unchecked(&api_url)
    }

    /// Fetches the nonce associated with a given public key.
    /// Returns an error if the HTTP request fails or the response cannot be parsed.
    /// If nonce is not found, it will return 0.
    pub async fn get_nonce_for_public_key<S: sov_modules_api::Spec>(
        &self,
        pub_key: &<S::CryptoSpec as CryptoSpec>::PublicKey,
    ) -> anyhow::Result<u64> {
        let credential_id = pub_key.credential_id();
        let nonce_url = format!(
            "{}/modules/nonces/state/nonces/items/{}",
            self.base_url, credential_id
        );
        let response = self.http_client.get(&nonce_url).send().await?;

        let nonce = match response.error_for_status() {
            Ok(res) => res.json::<NonceResponse>().await?.value,
            Err(_) => 0,
        };

        tracing::debug!(url = nonce_url, ?nonce, "Queried nonce");

        Ok(nonce)
    }

    /// Getting [`TokenId`] from given parameters.
    pub async fn get_token_id<S: sov_modules_api::Spec>(
        &self,
        token_name: &str,
        token_decimals: Option<u8>,
        deployer: &S::Address,
    ) -> anyhow::Result<TokenId> {
        let token_url = match token_decimals {
            Some(decimals) => format!(
                "{}/modules/bank/tokens?token_name={}&token_decimals={}&sender={}",
                self.base_url, token_name, decimals, deployer
            ),
            None => format!(
                "{}/modules/bank/tokens?token_name={}&sender={}",
                self.base_url, token_name, deployer
            ),
        };
        tracing::debug!(url = token_url, "Querying token_id");

        let response = self.http_client.get(token_url).send().await?;
        let response = response.json::<TokenIdResponse>().await?;

        Ok(response.token_id)
    }

    async fn query_amount(&self, url: &str) -> anyhow::Result<Amount> {
        let response = self.http_client.get(url).send().await?.error_for_status()?;
        let response = response.json::<Coins>().await?;

        Ok(response.amount)
    }

    /// Get total supply of given sov-bank token
    pub async fn get_total_supply(&self, token_id: &TokenId) -> anyhow::Result<Amount> {
        let total_supply_url = format!(
            "{}/modules/bank/tokens/{}/total-supply",
            self.base_url, token_id
        );
        tracing::debug!("Querying total supply");

        self.query_amount(&total_supply_url).await
    }

    /// Get list of admins for given token.
    pub async fn get_admins<S: sov_modules_api::Spec>(
        &self,
        token_id: &TokenId,
    ) -> anyhow::Result<Vec<TokenHolder<S>>> {
        let url = format!("{}/modules/bank/tokens/{}/admins", self.base_url, token_id);

        let response = self.http_client.get(url).send().await?;
        let response = response.json::<AdminsResponse<S>>().await?;

        Ok(response.admins)
    }

    /// Get balance of the user.
    pub async fn get_balance<S: sov_modules_api::Spec>(
        &self,
        account_address: &S::Address,
        token_id: &TokenId,
        rollup_height: Option<u64>,
    ) -> anyhow::Result<Amount> {
        let height_param: String = rollup_height
            .map(|h| format!("?rollup_height={h}"))
            .unwrap_or_default();
        let balance_url = format!(
            "{}/modules/bank/tokens/{}/balances/{}{}",
            self.base_url, token_id, account_address, height_param
        );
        let amount = self.query_amount(&balance_url).await?;

        tracing::debug!(
            address = %account_address,
            url = balance_url,
            %amount,
            "Queried balance",
        );

        Ok(amount)
    }

    /// Send transactions to the sequencer.
    /// Accepts vector of borsh serialized [`sov_modules_api::transaction::Transaction`].
    /// Returns batch submission receipt and hashes of transactions provided to this method.
    /// If `wait_for_processing` is set to true,
    /// it will wait for the first transaction to become processed or finalized.
    /// (!) If automatic batch production is not enabled
    pub async fn send_transactions_to_sequencer(
        &self,
        raw_txs: Vec<Vec<u8>>,
        wait_for_processing: bool,
    ) -> anyhow::Result<Vec<types::TxHash>> {
        let txs_included = raw_txs.len();
        tracing::info!(
            txs_included = txs_included,
            "Calling `publish_batch` sequencer endpoint"
        );

        let mut tx_hashes = Vec::with_capacity(raw_txs.len());
        for tx in raw_txs {
            let value = self
                .client
                .accept_tx(&AcceptTxBody {
                    body: BASE64_STANDARD.encode(tx),
                })
                .await
                .context("Failed to submit tx")?;
            let tx_hash = value.id.clone();
            tracing::info!(hash = tx_hash.as_str(), "Submitted tx");
            tx_hashes.push(tx_hash);
        }

        if wait_for_processing {
            // We pick the first tx hash of the batch, any would work.
            // Ideally we should wait for all.
            let Some(tx_hash_to_wait) = tx_hashes.first() else {
                return Ok(tx_hashes);
            };
            self.wait_for_tx_processing(tx_hash_to_wait).await?;
        }
        Ok(tx_hashes)
    }

    /// Waits for transactions to become processed or finalized.
    /// Timeout is 5 minutes.
    pub async fn wait_for_tx_processing(&self, tx_hash: &types::TxHash) -> anyhow::Result<()> {
        let max_waiting_time = Duration::from_secs(300);
        tracing::info!(?max_waiting_time, "Going to wait for batch to be processed");
        let start_wait = Instant::now();

        let mut subscription = self
            .client
            .subscribe_to_tx_status_updates(tx_hash.parse()?)
            .await?;

        while start_wait.elapsed() < max_waiting_time {
            if let Some(tx_info) = subscription.next().await.transpose()? {
                if tx_info.status == types::TxStatus::Processed
                    || tx_info.status == types::TxStatus::Finalized
                {
                    tracing::info!("Rollup has processed the submitted batch!");
                    return Ok(());
                }
            }
        }
        anyhow::bail!(
            "Giving up waiting for target batch to be published after {:?}",
            start_wait.elapsed()
        );
    }

    /// Performs a get request at given URL on the REST API socket.
    pub async fn query_rest_endpoint<R: serde::de::DeserializeOwned>(
        &self,
        url: &str,
    ) -> anyhow::Result<R> {
        let url = format!("{}{}", self.base_url, url);
        let response = self.http_client.get(url).send().await?;
        let data = response.json::<R>().await?;
        Ok(data)
    }

    /// HTTP GET to the given endpoint, returning plain text.
    pub async fn http_get(&self, url: &str) -> anyhow::Result<String> {
        let url = format!("{}{}", self.base_url, url);
        Ok(self.http_client.get(url).send().await?.text().await?)
    }

    /// HTTP POST to the given endpoint, returning plain text.
    pub async fn http_post(&self, url: &str) -> anyhow::Result<String> {
        let url = format!("{}{}", self.base_url, url);
        Ok(self.http_client.post(url).send().await?.text().await?)
    }

    /// Requests if given DA address is allowed sequencer.
    /// Returns balance as well.
    pub async fn sequencer_rollup_address<S: sov_modules_api::Spec, Da: DaSpec>(
        &self,
        da_address: &Da::Address,
    ) -> anyhow::Result<Option<KnownSequencer<S>>> {
        let url = format!(
            "{}/modules/sequencer-registry/state/known-sequencers/items/{}",
            self.base_url, &da_address,
        );

        let response = self.http_client.get(url).send().await?;
        if !response.status().is_success() {
            anyhow::bail!("Unsuccessful response {:?}", response);
        }
        let response = response
            .json::<KnownSequencerResponse<S>>()
            .await
            .context("Deserialization of `KnownSequencerResponse`")?;

        Ok(Some(response.value))
    }
}

#[derive(serde::Deserialize)]
struct ModuleInfo {
    #[allow(dead_code)]
    id: String,
}

#[derive(serde::Deserialize)]
struct ModulesList {
    modules: HashMap<String, ModuleInfo>,
}

/// Call to list of modules endpoint and checking if all modules are listed there.
/// It assumes that "bank", "accounts" and "nonces" are standard Sovereign modules.
async fn check_if_rollup_has_standard_modules(
    client: &reqwest::Client,
    base_url: &str,
) -> anyhow::Result<bool> {
    let url = format!("{base_url}/modules");
    let response = client.get(&url).send().await?;
    let module_response: ModulesList = response.json().await?;

    Ok(module_response.modules.contains_key("bank")
        && module_response.modules.contains_key("accounts")
        && module_response.modules.contains_key("uniqueness")
        && module_response.modules.contains_key("sequencer-registry"))
}
