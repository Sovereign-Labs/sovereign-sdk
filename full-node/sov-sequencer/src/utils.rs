use borsh::BorshSerialize;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};

/// A simple client for the sequencer RPC.
pub struct SimpleClient {
    client: HttpClient,
}

impl SimpleClient {
    /// Creates a new client at the given endpoint
    pub fn new(endpoint: &str) -> Self {
        let client = HttpClientBuilder::default().build(endpoint).unwrap();
        Self { client }
    }

    /// Sends a transaction to the sequencer for immediate publication
    pub async fn send_transaction<Tx: BorshSerialize>(&self, tx: Tx) -> Result<(), anyhow::Error> {
        let batch = vec![tx.try_to_vec()?];
        let response: String = self.client.request("sequencer_publishBatch", batch).await?;
        println!("response: {:?}", response);
        Ok(())
    }

    /// Get a reference to the underlying [`HttpClient`]
    pub fn inner(&self) -> &HttpClient {
        &self.client
    }
}

use jsonrpsee::types::ErrorObjectOwned;

/// Creates an jsonrpsee ErrorObject
pub fn to_jsonrpsee_error_object(err: impl ToString, message: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        Some(err.to_string()),
    )
}
