use borsh::BorshSerialize;
use jsonrpsee::core::client::ClientT;
use jsonrpsee::http_client::{HttpClient, HttpClientBuilder};
use jsonrpsee::types::ErrorObjectOwned;
use jsonrpsee::ws_client::{WsClient, WsClientBuilder};

/// A simple client for the sequencer RPC.
pub struct SimpleClient {
    http_client: HttpClient,
    ws_client: WsClient,
}

impl SimpleClient {
    /// Creates a new client at the given endpoint
    pub async fn new(address: &str, port: u16) -> Result<Self, anyhow::Error> {
        let http_client = HttpClientBuilder::default()
            .build(format!("http://{address}:{port}"))
            .unwrap();
        let ws_client = WsClientBuilder::default()
            .build(&format!("ws://{address}:{port}"))
            .await?;
        Ok(Self {
            http_client,
            ws_client,
        })
    }

    /// Sends a transaction to the sequencer for immediate publication.
    pub async fn send_transaction<Tx: BorshSerialize>(&self, tx: Tx) -> Result<(), anyhow::Error> {
        let batch = vec![tx.try_to_vec()?];

        let response: String = self
            .http_client
            .request("sequencer_publishBatch", batch)
            .await?;
        println!("response: {:?}", response);
        Ok(())
    }

    /// Get a reference to the underlying [`HttpClient`]
    pub fn http(&self) -> &HttpClient {
        &self.http_client
    }

    /// Get a reference to the underlying [`WsClient`]
    pub fn ws(&self) -> &WsClient {
        &self.ws_client
    }
}

/// Creates an jsonrpsee ErrorObject
pub fn to_jsonrpsee_error_object(err: impl ToString, message: &str) -> ErrorObjectOwned {
    ErrorObjectOwned::owned(
        jsonrpsee::types::error::UNKNOWN_ERROR_CODE,
        message,
        Some(err.to_string()),
    )
}
