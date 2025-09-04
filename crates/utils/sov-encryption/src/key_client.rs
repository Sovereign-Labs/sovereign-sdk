use async_trait::async_trait;

use crate::{
    config::KeyClientConfig,
    error::EncryptionError,
};

#[cfg(feature = "unix-client")]
use {
    std::time::{Duration, SystemTime, UNIX_EPOCH},
    tracing::{debug, warn},
    crate::config::{KeyRequest, KeyResponse, KeyType},
};

#[async_trait]
pub trait KeyClient: Send + Sync {
    async fn get_encryption_key(&self) -> Result<Vec<u8>, EncryptionError>;
    async fn get_decryption_key(&self, key_id: Option<String>) -> Result<Vec<u8>, EncryptionError>;
}

/// Create a key client from configuration
pub fn create_key_client(config: KeyClientConfig) -> Box<dyn KeyClient> {
    match config {
        KeyClientConfig::Static { encryption_key, decryption_key } => {
            Box::new(StaticKeyClient::new(encryption_key, decryption_key))
        }
        #[cfg(feature = "unix-client")]
        KeyClientConfig::UnixSocket { socket_path, timeout, max_retries, retry_delay_ms } => {
            Box::new(UnixSocketKeyClient::new(socket_path, timeout, max_retries, retry_delay_ms))
        }
    }
}

/// Static key client that uses pre-configured keys (no external fetching)
pub struct StaticKeyClient {
    encryption_key: Vec<u8>,
    decryption_key: Vec<u8>,
}

impl StaticKeyClient {
    pub fn new(encryption_key: String, decryption_key: Option<String>) -> Self {
        let enc_key = hex::decode(&encryption_key)
            .expect("Invalid hex encryption key");
        
        // Validate key length for AES-256 (32 bytes)
        if enc_key.len() != 32 {
            panic!("Invalid hex encryption key");
        }
        
        let dec_key = match decryption_key {
            Some(dk) => {
                let key = hex::decode(&dk).expect("Invalid hex decryption key");
                if key.len() != 32 {
                    panic!("Invalid hex decryption key");
                }
                key
            },
            None => enc_key.clone(),
        };
        
        Self {
            encryption_key: enc_key,
            decryption_key: dec_key,
        }
    }
}

#[async_trait]
impl KeyClient for StaticKeyClient {
    async fn get_encryption_key(&self) -> Result<Vec<u8>, EncryptionError> {
        Ok(self.encryption_key.clone())
    }

    async fn get_decryption_key(&self, _key_id: Option<String>) -> Result<Vec<u8>, EncryptionError> {
        Ok(self.decryption_key.clone())
    }
}

// Unix Socket Key Client (only available with unix-client feature)
#[cfg(feature = "unix-client")]
pub struct UnixSocketKeyClient {
    socket_path: std::path::PathBuf,
    timeout: Duration,
    max_retries: u32,
    retry_delay: Duration,
}

#[cfg(feature = "unix-client")]
impl UnixSocketKeyClient {
    pub fn new(
        socket_path: std::path::PathBuf,
        timeout_secs: u64,
        max_retries: u32,
        retry_delay_ms: u64,
    ) -> Self {
        Self {
            socket_path,
            timeout: Duration::from_secs(timeout_secs),
            max_retries,
            retry_delay: Duration::from_millis(retry_delay_ms),
        }
    }

    async fn request_key(&self, key_type: KeyType, key_id: Option<String>) -> Result<KeyResponse, EncryptionError> {
        let mut retries = 0;

        loop {
            match self.try_request_key(key_type.clone(), key_id.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) if e.is_retryable() && retries < self.max_retries => {
                    warn!(
                        error = %e,
                        retry = retries + 1,
                        max_retries = self.max_retries,
                        "Key request failed, retrying..."
                    );
                    retries += 1;
                    tokio::time::sleep(self.retry_delay).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn try_request_key(&self, key_type: KeyType, key_id: Option<String>) -> Result<KeyResponse, EncryptionError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::UnixStream;
        use tokio::time::timeout;

        debug!("Connecting to key server at {:?}", self.socket_path);
        
        let stream = timeout(
            self.timeout,
            UnixStream::connect(&self.socket_path)
        )
        .await
        .map_err(|_| EncryptionError::KeyServerConnection(
            std::io::Error::new(std::io::ErrorKind::TimedOut, "Connection timeout")
        ))?
        .map_err(EncryptionError::KeyServerConnection)?;

        let request = KeyRequest {
            key_type,
            key_id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };

        debug!("Sending key request: {:?}", request);
        self.send_request(stream, &request).await
    }

    async fn send_request(&self, mut stream: tokio::net::UnixStream, request: &KeyRequest) -> Result<KeyResponse, EncryptionError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::time::timeout;

        let request_json = serde_json::to_vec(request)?;
        let request_len = request_json.len() as u32;

        // Send length-prefixed request
        stream.write_all(&request_len.to_le_bytes()).await?;
        stream.write_all(&request_json).await?;
        stream.flush().await?;

        debug!("Request sent, waiting for response...");

        // Read length-prefixed response
        let mut len_bytes = [0u8; 4];
        timeout(self.timeout, stream.read_exact(&mut len_bytes))
            .await
            .map_err(|_| EncryptionError::KeyServerConnection(
                std::io::Error::new(std::io::ErrorKind::TimedOut, "Response timeout")
            ))??;

        let response_len = u32::from_le_bytes(len_bytes) as usize;
        if response_len > 1024 * 1024 {
            return Err(EncryptionError::KeyServerResponse(
                "Response too large".to_string()
            ));
        }

        let mut response_bytes = vec![0u8; response_len];
        timeout(self.timeout, stream.read_exact(&mut response_bytes))
            .await
            .map_err(|_| EncryptionError::KeyServerConnection(
                std::io::Error::new(std::io::ErrorKind::TimedOut, "Response timeout")
            ))??;

        let response: KeyResponse = serde_json::from_slice(&response_bytes)
            .map_err(|e| EncryptionError::KeyServerResponse(format!("Invalid JSON response: {}", e)))?;

        debug!("Received key response for key_id: {}", response.key_id);
        Ok(response)
    }
}

#[cfg(feature = "unix-client")]
#[async_trait]
impl KeyClient for UnixSocketKeyClient {
    async fn get_encryption_key(&self) -> Result<Vec<u8>, EncryptionError> {
        let response = self.request_key(KeyType::Encryption, None).await?;
        hex::decode(&response.key)
            .map_err(|e| EncryptionError::InvalidKeyFormat(format!("Invalid hex key: {}", e)))
    }

    async fn get_decryption_key(&self, key_id: Option<String>) -> Result<Vec<u8>, EncryptionError> {
        let response = self.request_key(KeyType::Decryption, key_id).await?;
        hex::decode(&response.key)
            .map_err(|e| EncryptionError::InvalidKeyFormat(format!("Invalid hex key: {}", e)))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_static_key_client() {
        let hex_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let client = StaticKeyClient::new(hex_key.to_string(), None);
        
        let enc_key = client.get_encryption_key().await.unwrap();
        let dec_key = client.get_decryption_key(None).await.unwrap();
        
        assert_eq!(enc_key.len(), 32);
        assert_eq!(enc_key, dec_key);
    }

    #[tokio::test]
    async fn test_static_key_client_different_keys() {
        let enc_hex = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let dec_hex = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
        
        let client = StaticKeyClient::new(enc_hex.to_string(), Some(dec_hex.to_string()));
        
        let enc_key = client.get_encryption_key().await.unwrap();
        let dec_key = client.get_decryption_key(None).await.unwrap();
        
        assert_eq!(enc_key.len(), 32);
        assert_eq!(dec_key.len(), 32);
        assert_ne!(enc_key, dec_key);
    }

    #[test]
    fn test_create_key_client_static() {
        let config = KeyClientConfig::Static {
            encryption_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            decryption_key: None,
        };
        
        let _client = create_key_client(config);
        // If we get here without panic, the client was created successfully
    }

    #[test]
    #[should_panic(expected = "Invalid hex encryption key")]
    fn test_static_key_client_invalid_hex() {
        StaticKeyClient::new("invalid_hex".to_string(), None);
    }

    #[test]
    fn test_create_key_client_with_different_keys() {
        let config = KeyClientConfig::Static {
            encryption_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            decryption_key: Some("fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210".to_string()),
        };
        
        let _client = create_key_client(config);
    }

    #[test]
    #[should_panic(expected = "Invalid hex encryption key")]
    fn test_static_key_client_wrong_length() {
        // 16 bytes instead of 32
        StaticKeyClient::new("0123456789abcdef0123456789abcdef".to_string(), None);
    }

    #[test]
    #[should_panic(expected = "Invalid hex decryption key")]
    fn test_static_key_client_invalid_decryption_key() {
        StaticKeyClient::new(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
            Some("invalid_hex".to_string())
        );
    }

    #[tokio::test]
    async fn test_static_key_client_key_id_ignored() {
        let hex_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let client = StaticKeyClient::new(hex_key.to_string(), None);
        
        // Key ID should be ignored for static client
        let key1 = client.get_decryption_key(None).await.unwrap();
        let key2 = client.get_decryption_key(Some("some_key_id".to_string())).await.unwrap();
        
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_static_key_client_hex_case_insensitive() {
        let lower_key = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
        let upper_key = "ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789";
        
        let client1 = StaticKeyClient::new(lower_key.to_string(), None);
        let client2 = StaticKeyClient::new(upper_key.to_string(), None);
        
        // Should create both clients successfully and have same keys
        assert_eq!(client1.encryption_key, client2.encryption_key);
    }
}