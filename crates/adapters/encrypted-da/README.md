# Encrypted DA Adapter

A generic encryption wrapper for Sovereign SDK DA services that provides transparent encryption/decryption of data stored on Data Availability layers. 

The wrapper implements the same `DaService` trait as the inner service, making it a drop-in replacement.

## Overview

The `sov-encrypted-da` crate provides `EncryptedDaService<T>`, a wrapper that adds encryption capabilities to any DA service implementing the `DaService` trait. This allows you to encrypt data before it's submitted to the DA layer and decrypt it when reading, while maintaining full compatibility with existing rollup code.

# TODO CHANGE THIS
## Key Features

- **🔐 Transparent Encryption**: Encrypts data before DA submission, decrypts on retrieval
- **🔄 Universal Compatibility**: Works with any DA service (MockDA, Celestia, Avail, etc.)
- **🛡️ Unix Socket Key Management**: Secure key fetching via Unix domain sockets  
- **⚙️ Configuration-Driven**: Enable/disable encryption without code changes
- **🧪 Testable**: Mock encryption layers for testing
- **🚀 Zero Breaking Changes**: Existing code works unchanged

## Usage

### Basic Usage

```rust
use sov_encrypted_da::EncryptedDaService;
use sov_encryption::{EncryptionConfig, CipherType};
use sov_mock_da::{MockDaConfig, storable::StorableMockDaService};

// Create encryption config
let encryption_config = EncryptionConfig {
    socket_path: "/tmp/keys.sock".into(),
    cipher_type: CipherType::Aes256Gcm,
    key_server_timeout: 30,
    key_rotation_interval: Some(3600),
    max_retries: 3,
    retry_delay_ms: 1000,
};

// Create DA service config
let da_config = MockDaConfig {
    sender_address: MockAddress::new([1; 32]),
    finalization_blocks: 2,
    block_producing: BlockProducingConfig::Manual,
    connection_string: ":memory:".to_string(),
    da_layer: None,
    randomization: None,
};

// Method 1: Factory method (recommended)
let encrypted_da = EncryptedDaService::<StorableMockDaService>::from_configs_with_shutdown(
    da_config,
    encryption_config, 
    shutdown_rx
).await?;

// Method 2: Manual construction
let mock_da = StorableMockDaService::from_config(da_config, shutdown_rx).await?;
let encrypted_da = EncryptedDaService::new(mock_da, encryption_config);

// Use exactly like any other DA service!
let receipt = encrypted_da.send_transaction(b"my data").await.await?;
let block = encrypted_da.get_block_at(height).await?;
```

### Integration with Rollups

```rust
// In your rollup configuration
pub struct RollupConfig {
    pub da_layer: DaLayerConfig,
    pub encryption: Option<EncryptionConfig>, // Add this field
}

// Factory function
pub async fn create_da_service(config: &RollupConfig) -> Box<dyn DaService> {
    let base_service: Box<dyn DaService> = match &config.da_layer {
        DaLayerConfig::MockDa(mock_config) => {
            Box::new(StorableMockDaService::from_config(mock_config.clone(), shutdown_rx).await?)
        }
        DaLayerConfig::Celestia(celestia_config) => {
            Box::new(CelestiaService::new(celestia_config.clone()).await?)
        }
    };
    
    // Wrap with encryption if configured
    match &config.encryption {
        Some(enc_config) => {
            Box::new(EncryptedDaService::new(base_service, enc_config.clone())?)
        }
        None => base_service,
    }
}

// Your existing rollup code works unchanged!
let da_service = create_da_service(&config).await?;
let rollup = RollupRunner::new(da_service, state_manager, prover);
rollup.run().await?;
```

### Configuration Example

TOML configuration:
```toml
[rollup]
da_layer = "mock-da"

[rollup.mock_da]
sender_address = "0x1234..."
finalization_blocks = 2
block_producing = "manual"

[rollup.encryption]
socket_path = "/var/run/encryption/keys.sock"
cipher_type = "aes256-gcm"  
key_server_timeout = 30
key_rotation_interval = 3600
max_retries = 3
retry_delay_ms = 1000
```

## Key Management

The encrypted DA adapter fetches encryption keys via Unix domain sockets. You need to run a key server that:

1. Listens on a Unix socket (e.g., `/tmp/keys.sock`)
2. Accepts JSON requests for encryption/decryption keys
3. Returns hex-encoded AES-256 keys

### Key Server Protocol

**Request Format:**
```json
{
  "key_type": "encryption", // or "decryption" 
  "key_id": null,           // optional key identifier
  "timestamp": 1234567890   // unix timestamp
}
```

**Response Format:**
```json
{
  "key": "0123456789abcdef...", // 64-char hex string (32 bytes)
  "key_id": "key_20240101_001", 
  "expires_at": 1234567890       // optional expiration
}
```

### Example Key Server

A simple key server implementation:

```rust
use tokio::net::UnixListener;
use sov_encryption::{KeyRequest, KeyResponse};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = UnixListener::bind("/tmp/keys.sock")?;
    
    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_client(stream));
    }
    
    Ok(())
}

async fn handle_client(mut stream: UnixStream) -> Result<(), Box<dyn std::error::Error>> {
    // Read request length
    let mut len_bytes = [0u8; 4];
    stream.read_exact(&mut len_bytes).await?;
    let len = u32::from_le_bytes(len_bytes);
    
    // Read request
    let mut req_bytes = vec![0u8; len as usize];
    stream.read_exact(&mut req_bytes).await?;
    let request: KeyRequest = serde_json::from_slice(&req_bytes)?;
    
    // Generate response (in production, fetch from secure key store)
    let response = KeyResponse {
        key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        key_id: format!("key_{}", chrono::Utc::now().timestamp()),
        expires_at: Some(chrono::Utc::now().timestamp() + 3600),
    };
    
    // Send response
    let resp_bytes = serde_json::to_vec(&response)?;
    stream.write_all(&(resp_bytes.len() as u32).to_le_bytes()).await?;
    stream.write_all(&resp_bytes).await?;
    
    Ok(())
}
```

## Security Considerations

- **Key Management**: Implement secure key storage and rotation
- **Unix Socket Permissions**: Restrict socket file permissions (e.g., `600`)
- **Key Rotation**: Configure appropriate rotation intervals
- **Audit Logging**: Log key requests and encryption operations
- **Backup Keys**: Ensure encrypted data can be decrypted during key rotation
