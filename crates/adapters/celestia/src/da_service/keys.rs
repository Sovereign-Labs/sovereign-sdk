//! Vibe-coded helpers for loading private keys into celestia_client::Client
use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use sha2::{Digest, Sha256};
use xsalsa20poly1305::aead::{Aead, KeyInit};
use xsalsa20poly1305::{Nonce, XSalsa20Poly1305};

const BCRYPT_COST: u32 = 12;
const NONCE_SIZE: usize = 24;

/// Reads and decrypts a Tendermint private key from an armored key file.
///
/// The file format is ASCII armor with headers including KDF type and salt.
/// For bcrypt KDF, the decryption process is:
/// 1. Parse armor format to extract headers and encrypted data
/// 2. Derive key using bcrypt with the password and salt
/// 3. Hash the bcrypt output with SHA256 to get 32-byte key
/// 4. Decrypt using XSalsa20-Poly1305 (nonce is first 24 bytes)
///
/// # Arguments
/// * `armor_str` - The armored key file content
/// * `password` - The password to decrypt the key
///
/// # Returns
/// The private key in hexadecimal format
pub fn read_tendermint_key_file(armor_str: &str, password: &str) -> Result<String> {
    // Parse the armor format
    let (headers, encrypted_data) = parse_armor(armor_str)?;

    // Validate KDF type
    let kdf = headers
        .get("kdf")
        .context("Missing 'kdf' header in armor")?;
    if kdf != "bcrypt" {
        bail!("Unsupported KDF type: {}, only bcrypt is supported", kdf);
    }

    // Extract and decode salt
    let salt_hex = headers
        .get("salt")
        .context("Missing 'salt' header in armor")?;
    let salt_vec = hex::decode(salt_hex).context("Failed to decode salt from hex")?;

    // Bcrypt requires exactly 16 bytes for salt
    if salt_vec.len() != 16 {
        bail!(
            "Invalid salt length: expected 16 bytes, got {}",
            salt_vec.len()
        );
    }
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&salt_vec);

    // Derive encryption key using bcrypt
    // The Cosmos SDK's custom bcrypt returns the full hash string (e.g., "$2a$12$...")
    // as bytes, which is then hashed with SHA256
    let bcrypt_hash = bcrypt::hash_with_salt(password, BCRYPT_COST, salt)
        .context("Failed to hash password with bcrypt")?;

    // Convert the bcrypt hash to 2a version format to match Tendermint's implementation
    // The Rust bcrypt library defaults to 2y, but Tendermint uses 2a
    let bcrypt_hash_string = bcrypt_hash.format_for_version(bcrypt::Version::TwoA);
    let bcrypt_hash_bytes = bcrypt_hash_string.as_bytes();

    // Hash the bcrypt output with SHA256 to get 32-byte key
    let mut hasher = Sha256::new();
    hasher.update(bcrypt_hash_bytes);
    let encryption_key = hasher.finalize();

    // Decrypt using XSalsa20-Poly1305
    if encrypted_data.len() <= NONCE_SIZE {
        bail!(
            "Encrypted data too short: expected > {} bytes, got {}",
            NONCE_SIZE,
            encrypted_data.len()
        );
    }

    // Extract nonce (first 24 bytes)
    let nonce_bytes = &encrypted_data[..NONCE_SIZE];
    let nonce = Nonce::from_slice(nonce_bytes);

    // Extract ciphertext (remaining bytes)
    let ciphertext = &encrypted_data[NONCE_SIZE..];

    // Create cipher and decrypt
    let cipher = XSalsa20Poly1305::new(encryption_key.as_slice().into());
    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|_| anyhow::anyhow!("Decryption failed - wrong password or corrupted data"))?;

    // Decode Amino encoding to extract the raw private key
    // Amino format: <4-byte prefix><1-byte length><raw key bytes>
    // For PrivKeySecp256k1: prefix is 0xE1B0F79B, length is 0x20 (32 bytes)
    const AMINO_PREFIX_LEN: usize = 4;
    const AMINO_LENGTH_LEN: usize = 1;
    const AMINO_HEADER_LEN: usize = AMINO_PREFIX_LEN + AMINO_LENGTH_LEN;
    const EXPECTED_KEY_LEN: usize = 32;

    if plaintext.len() < AMINO_HEADER_LEN + EXPECTED_KEY_LEN {
        bail!(
            "Decrypted data too short: expected at least {} bytes, got {}",
            AMINO_HEADER_LEN + EXPECTED_KEY_LEN,
            plaintext.len()
        );
    }

    // Verify Amino prefix for PrivKeySecp256k1
    const AMINO_PREFIX: [u8; 4] = [0xE1, 0xB0, 0xF7, 0x9B];
    if plaintext[..AMINO_PREFIX_LEN] != AMINO_PREFIX {
        bail!(
            "Invalid Amino prefix: expected {:02x?}, got {:02x?}",
            AMINO_PREFIX,
            &plaintext[..AMINO_PREFIX_LEN]
        );
    }

    // Verify length byte
    let key_len = plaintext[AMINO_PREFIX_LEN] as usize;
    if key_len != EXPECTED_KEY_LEN {
        bail!(
            "Invalid key length in Amino encoding: expected {}, got {}",
            EXPECTED_KEY_LEN,
            key_len
        );
    }

    // Extract the raw 32-byte private key
    let private_key = &plaintext[AMINO_HEADER_LEN..AMINO_HEADER_LEN + EXPECTED_KEY_LEN];

    // Convert to hex
    Ok(hex::encode(private_key))
}

/// Parses ASCII armor format and extracts headers and base64-decoded data
fn parse_armor(armor_str: &str) -> Result<(std::collections::HashMap<String, String>, Vec<u8>)> {
    let lines: Vec<&str> = armor_str.lines().collect();

    // Find armor boundaries
    let start_marker = "-----BEGIN TENDERMINT PRIVATE KEY-----";
    let end_marker = "-----END TENDERMINT PRIVATE KEY-----";

    let start_idx = lines
        .iter()
        .position(|line| line.trim() == start_marker)
        .context("Missing BEGIN marker in armor")?;

    let end_idx = lines
        .iter()
        .position(|line| line.trim() == end_marker)
        .context("Missing END marker in armor")?;

    if end_idx <= start_idx {
        bail!("Invalid armor format: END marker before BEGIN marker");
    }

    // Parse headers (key: value format)
    let mut headers = std::collections::HashMap::new();
    let mut data_lines = Vec::new();
    let mut parsing_headers = true;

    for line in &lines[start_idx + 1..end_idx] {
        let line = line.trim();
        if line.is_empty() {
            parsing_headers = false;
            continue;
        }

        if parsing_headers {
            if let Some((key, value)) = line.split_once(':') {
                headers.insert(key.trim().to_string(), value.trim().to_string());
            }
        } else {
            // Skip OpenPGP CRC24 checksum lines (start with '=')
            if !line.starts_with('=') {
                data_lines.push(line);
            }
        }
    }

    // Decode base64 data
    let data_str = data_lines.join("");
    let data = BASE64
        .decode(data_str.as_bytes())
        .context("Failed to decode base64 data")?;

    Ok((headers, data))
}

pub fn seed_phrase_to_private_key_cosmos(mnemonic_str: &str) -> anyhow::Result<String> {
    use bip32::{DerivationPath as Bip32Path, XPrv};

    let mnemonic = bip39::Mnemonic::parse_normalized(mnemonic_str)
        .context(format!("Failed to parse mnemonic: '{mnemonic_str}'"))?;

    let seed = mnemonic.to_seed("");
    let path = "m/44'/118'/0'/0/0".parse::<Bip32Path>().context("path")?;
    let xprv = XPrv::derive_from_path(seed, &path).context("private key")?;
    let private_key_hex = hex::encode(xprv.private_key().to_bytes());

    Ok(private_key_hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_armor() {
        let armor = include_str!("../../../../../docker/credentials/bridge-0.key");

        let result = parse_armor(armor);
        assert!(result.is_ok(), "Failed: {result:?}");

        let (headers, data) = result.unwrap();
        assert_eq!(headers.get("kdf").unwrap(), "bcrypt");
        assert_eq!(
            headers.get("salt").unwrap(),
            "68B0092F4FC5386C20DA96ECEE1BFE09"
        );
        assert_eq!(headers.get("type").unwrap(), "secp256k1");
        assert!(!data.is_empty());
    }
}
