# sov-encryption

A modular encryption utility for Sovereign SDK that provides AES-256-GCM encryption with flexible key management options.

## Overview

The `sov-encryption` crate enables secure encryption and decryption of data with multiple key fetching strategies. It's designed to be modular, allowing you to use only the features you need while maintaining strong security practices.

## Features

- **🔐 AES-256-GCM Encryption**: Industry-standard authenticated encryption
- **🔑 Flexible Key Management**: Multiple key client implementations
- **🛠️ Modular Design**: Feature flags for optional functionality
- **⚡ Minimal Dependencies**: Core encryption works without networking
- **🔄 Key Rotation Support**: Optional automatic key rotation
- **🛡️ Security First**: Secure key fetching with retry logic and timeouts

### Feature Flags

- `aes-encryption` (default): Core AES-256-GCM encryption functionality
- `unix-client`: Unix domain socket key client for secure local key fetching

## Key Management Options

### 1. Static Keys (Simplest)

Use pre-configured keys stored directly in configuration:

```rust
use sov_encryption::{EncryptionConfig, KeyClientConfig, CipherType};

let config = EncryptionConfig {
    key_client: KeyClientConfig::Static {
        encryption_key: "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        decryption_key: None, // Uses same key for both if not specified
    },
    cipher_type: CipherType::Aes256Gcm,
    key_rotation_interval: None,
};
```

### 2. Unix Socket Key Client (Secure Local)

Fetch keys from a local key server via Unix domain sockets:

```rust
use sov_encryption::{EncryptionConfig, KeyClientConfig, CipherType};
use std::path::PathBuf;

let config = EncryptionConfig {
    key_client: KeyClientConfig::UnixSocket {
        socket_path: PathBuf::from("/tmp/sovereign-keys.sock"),
        timeout: 30,
        max_retries: 3,
        retry_delay_ms: 1000,
    },
    cipher_type: CipherType::Aes256Gcm,
    key_rotation_interval: Some(3600), // Rotate keys every hour
};
```

## Usage Examples

### Basic Encryption Layer

```rust
use sov_encryption::{EncryptionLayer, EncryptionConfig, KeyClientConfig, CipherType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Configure encryption with static keys
    let config = EncryptionConfig {
        key_client: KeyClientConfig::Static {
            encryption_key: "your_64_char_hex_key_here".to_string(),
            decryption_key: None,
        },
        cipher_type: CipherType::Aes256Gcm,
        key_rotation_interval: None,
    };

    // Create encryption layer
    let encryption = EncryptionLayer::new(config).await?;

    // Encrypt data
    let plaintext = b"Hello, Sovereign!";
    let (ciphertext, key_id) = encryption.encrypt(plaintext).await?;
    println!("Encrypted {} bytes", ciphertext.len());

    // Decrypt data
    let decrypted = encryption.decrypt(&ciphertext, key_id.as_deref()).await?;
    assert_eq!(plaintext, &decrypted[..]);
    println!("Decryption successful!");

    Ok(())
}
```

### Using with Unix Socket Key Server

```rust
#[cfg(feature = "unix-client")]
use sov_encryption::{EncryptionLayer, EncryptionConfig, KeyClientConfig, CipherType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = EncryptionConfig {
        key_client: KeyClientConfig::UnixSocket {
            socket_path: "/var/run/sovereign/keys.sock".into(),
            timeout: 30,
            max_retries: 3,
            retry_delay_ms: 1000,
        },
        cipher_type: CipherType::Aes256Gcm,
        key_rotation_interval: Some(1800), // 30 minutes
    };

    let encryption = EncryptionLayer::new(config).await?;

    // Keys are fetched automatically from the Unix socket
    let (ciphertext, key_id) = encryption.encrypt(b"Secure data").await?;
    let decrypted = encryption.decrypt(&ciphertext, key_id.as_deref()).await?;

    Ok(())
}
```

### Custom Key Client

Implement your own key client for custom key fetching logic:

```rust
use sov_encryption::{KeyClient, EncryptionError};
use async_trait::async_trait;

struct CustomKeyClient {
    // Your custom fields
}

#[async_trait]
impl KeyClient for CustomKeyClient {
    async fn get_encryption_key(&self) -> Result<Vec<u8>, EncryptionError> {
        // Your custom key fetching logic
        Ok(vec![0u8; 32]) // Return 32-byte key
    }

    async fn get_decryption_key(&self, key_id: Option<String>) -> Result<Vec<u8>, EncryptionError> {
        // Your custom key fetching logic
        Ok(vec![0u8; 32]) // Return 32-byte key
    }
}
```

## Configuration

### TOML Configuration Examples

```toml
# Static key encryption (simplest)
[encryption]
cipher_type = "aes256-gcm"

[encryption.key_client]
type = "static"
encryption_key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
# decryption_key = "optional_different_key_for_decryption"

# Unix socket for secure local key management
[encryption.key_client]
type = "unix_socket"
socket_path = "/tmp/sovereign-keys.sock"
timeout = 30
max_retries = 3
retry_delay_ms = 1000
```

### Environment Variables

You can also configure via environment variables:

```bash
# Static key configuration
export SOV_ENCRYPTION_TYPE="static"
export SOV_ENCRYPTION_KEY="your_hex_key_here"

# Unix socket configuration
export SOV_ENCRYPTION_TYPE="unix_socket"
export SOV_ENCRYPTION_SOCKET_PATH="/tmp/keys.sock"
export SOV_ENCRYPTION_TIMEOUT="30"
```

## Unix Socket Key Server Protocol

The Unix socket key client communicates using a simple length-prefixed JSON protocol:

### Request Format
```json
{
  "key_type": "encryption", // or "decryption"
  "key_id": "optional_key_identifier",
  "timestamp": 1640995200
}
```

### Response Format
```json
{
  "key": "hex_encoded_32_byte_key",
  "key_id": "unique_key_identifier",
  "expires_at": 1640998800 // optional unix timestamp
}
```

### Example Key Server Implementation

```python
#!/usr/bin/env python3
import socket
import json
import struct
import os

def handle_key_request(conn):
    # Read length prefix (4 bytes, little endian)
    length_data = conn.recv(4)
    if len(length_data) != 4:
        return
    
    length = struct.unpack('<I', length_data)[0]
    
    # Read JSON request
    request_data = conn.recv(length)
    request = json.loads(request_data.decode('utf-8'))
    
    # Generate or fetch key (implement your logic here)
    key = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    
    response = {
        "key": key,
        "key_id": f"key_{request['timestamp']}",
        "expires_at": request['timestamp'] + 3600  # 1 hour from now
    }
    
    # Send length-prefixed response
    response_json = json.dumps(response).encode('utf-8')
    conn.send(struct.pack('<I', len(response_json)))
    conn.send(response_json)

# Create Unix socket server
sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
sock.bind('/tmp/sovereign-keys.sock')
sock.listen(1)

while True:
    conn, _ = sock.accept()
    try:
        handle_key_request(conn)
    finally:
        conn.close()
```

## Security Considerations

### Key Security
- **Static Keys**: Store securely and rotate regularly
- **Unix Sockets**: Ensure proper file permissions (600 or 700)
- **Key Rotation**: Enable automatic rotation for enhanced security

### Best Practices
1. **Use Unix sockets for production**: More secure than static keys
2. **Implement proper key rotation**: Regular key updates reduce exposure
3. **Secure key storage**: Never log or expose keys in plaintext
4. **Monitor key access**: Log key requests for security auditing
5. **Use strong keys**: Always use cryptographically secure random keys

### File Permissions
```bash
# Secure Unix socket permissions
chmod 600 /tmp/sovereign-keys.sock
chown sovereign:sovereign /tmp/sovereign-keys.sock
```

## Integration with Sovereign SDK

This crate is designed to integrate seamlessly with Sovereign SDK's DA (Data Availability) services:

```rust
use sov_encrypted_da::EncryptedDaService;
use sov_encryption::{EncryptionConfig, KeyClientConfig, CipherType};

// Create encrypted DA service
let encryption_config = EncryptionConfig {
    key_client: KeyClientConfig::UnixSocket {
        socket_path: "/var/run/sovereign/keys.sock".into(),
        timeout: 30,
        max_retries: 3,
        retry_delay_ms: 1000,
    },
    cipher_type: CipherType::Aes256Gcm,
    key_rotation_interval: Some(3600),
};

let encrypted_da = EncryptedDaService::new(base_da_service, encryption_config).await?;
```

## Error Handling

The crate provides comprehensive error types:

```rust
use sov_encryption::EncryptionError;

match encryption_result {
    Ok(data) => println!("Success: {:?}", data),
    Err(EncryptionError::KeyServerConnection(e)) => {
        eprintln!("Key server connection failed: {}", e);
    },
    Err(EncryptionError::InvalidKeyFormat(e)) => {
        eprintln!("Invalid key format: {}", e);
    },
    Err(EncryptionError::EncryptionFailed(e)) => {
        eprintln!("Encryption failed: {}", e);
    },
    // ... handle other error types
}
```

## Development

### Running Tests

```bash
# Test core encryption
cargo test

# Test with Unix client
cargo test --features unix-client

# Run examples
cargo run --example basic_usage --features unix-client
```