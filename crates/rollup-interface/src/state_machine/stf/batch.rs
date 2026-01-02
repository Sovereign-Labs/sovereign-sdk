//! Transaction batch types and utilities.
//!
//! This module defines the core transaction types used throughout the Sovereign SDK,
//! including fully authenticated transactions ready for DA layer submission.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};
use serde_with::serde_as;

use crate::Bytes;

/// Maximum allowed size for a raw transaction in bytes (1MB default)
pub const MAX_RAW_TX_SIZE: usize = 1 * 1024 * 1024;

/// Maximum allowed size for a fully baked transaction in bytes (2MB default)
pub const MAX_FULLY_BAKED_TX_SIZE: usize = 2 * 1024 * 1024;

/// `FullyBakedTx` represents a serialized signed rollup transaction that has been encoded with
/// authentication information and is ready to be placed on the DA layer.
#[serde_as]
#[derive(
    PartialEq,
    Eq,
    Clone,
    BorshSerialize,
    Serialize,
    Default,
    derive_more::AsRef,
)]
pub struct FullyBakedTx {
    /// Serialized transaction.
    #[as_ref(forward)]
    #[serde_as(as = "serde_with::base64::Base64")]
    pub data: Bytes,
    /// Sequencer-provided metadata for each transaction (e.g., timestamps).
    /// This data is NOT signed by users but is added by the sequencer.
    #[serde_as(as = "Option<serde_with::base64::Base64>")]
    pub sequencing_data: Option<Bytes>,
}

impl std::fmt::Debug for FullyBakedTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FullyBakedTx")
            .field("data", &hex::encode(&self.data))
            .finish()
    }
}

impl FullyBakedTx {
    /// Construct a `FullyBakedTx` containing the given data
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data: Bytes::from_owner(data),
            sequencing_data: None,
        }
    }

    /// Sets sequencing metadata
    pub fn set_sequencing_metadata(&mut self, metadata: &impl BorshSerialize) {
        self.sequencing_data = Some(borsh::to_vec(metadata).unwrap().into());
    }

    /// Returns the total serialized length of the transaction, including both
    /// the transaction data and sequencing metadata (if present).
    /// This is the length that will be sent on the DA layer.
    #[must_use]
    pub fn len(&self) -> usize {
        borsh::to_vec(self)
            .expect("Serialization to vec is infallible")
            .len()
    }

    /// Returns true if the transaction has no data
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty() && self.sequencing_data.is_none()
    }
}

/// `RawTx` represents a serialized signed rollup transaction. A `RawTx` needs to be encoded
/// with authentication information before being placed on the DA layer.
#[derive(
    PartialEq,
    Eq,
    Clone,
    BorshSerialize,
    Serialize,
    derive_more::AsRef,
)]
pub struct RawTx {
    /// Serialized transaction.
    #[as_ref(forward)]
    pub data: Vec<u8>,
}

impl std::fmt::Debug for RawTx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RawTx")
            .field("data", &hex::encode(&self.data))
            .finish()
    }
}

impl BorshDeserialize for RawTx {
    fn deserialize_reader<R: std::io::Read>(reader: &mut R) -> std::io::Result<Self> {
        // First read the length prefix (u32 in borsh for Vec<u8>)
        let len = u32::deserialize_reader(reader)? as usize;
        
        // Check if length exceeds maximum allowed size
        if len > MAX_RAW_TX_SIZE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("RawTx size {} exceeds maximum allowed size of {} bytes", len, MAX_RAW_TX_SIZE),
            ));
        }
        
        // Read the actual data
        let mut data = vec![0u8; len];
        reader.read_exact(&mut data)?;
        
        Ok(Self { data })
    }
}

impl<'de> Deserialize<'de> for RawTx {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawTxHelper {
            data: Vec<u8>,
        }
        
        let helper = RawTxHelper::deserialize(deserializer)?;
        
        if helper.data.len() > MAX_RAW_TX_SIZE {
            return Err(serde::de::Error::custom(format!(
                "RawTx size {} exceeds maximum allowed size of {} bytes",
                helper.data.len(),
                MAX_RAW_TX_SIZE
            )));
        }
        
        Ok(Self { data: helper.data })
    }
}

impl RawTx {
    /// Construct a `RawTx` containing the given data
    #[must_use]
    pub fn new(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Returns the length of the serialized transaction data
    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns true if the transaction has no data
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rawtx_borsh_within_limit() {
        // Create a transaction within the size limit
        let data = vec![1u8; 1024]; // 1KB
        let tx = RawTx::new(data.clone());
        
        // Serialize with borsh
        let serialized = borsh::to_vec(&tx).unwrap();
        
        // Deserialize should succeed
        let deserialized: RawTx = borsh::from_slice(&serialized).unwrap();
        assert_eq!(deserialized.data, data);
    }

    #[test]
    fn test_rawtx_borsh_exceeds_limit() {
        // Create a transaction that exceeds the size limit
        let data = vec![1u8; MAX_RAW_TX_SIZE + 1];
        let tx = RawTx::new(data);
        
        // Serialize with borsh
        let serialized = borsh::to_vec(&tx).unwrap();
        
        // Deserialize should fail
        let result: Result<RawTx, _> = borsh::from_slice(&serialized);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds maximum allowed size"));
    }

    #[test]
    fn test_rawtx_serde_json_within_limit() {
        // Create a transaction within the size limit
        let data = vec![1u8; 1024]; // 1KB
        let tx = RawTx::new(data.clone());
        
        // Serialize with serde_json
        let serialized = serde_json::to_string(&tx).unwrap();
        
        // Deserialize should succeed
        let deserialized: RawTx = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.data, data);
    }

    #[test]
    fn test_rawtx_serde_json_exceeds_limit() {
        // Create a transaction that exceeds the size limit
        let data = vec![1u8; MAX_RAW_TX_SIZE + 1];
        let tx = RawTx::new(data);
        
        // Serialize with serde_json
        let serialized = serde_json::to_string(&tx).unwrap();
        
        // Deserialize should fail
        let result: Result<RawTx, _> = serde_json::from_str(&serialized);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds maximum allowed size"));
    }

    #[test]
    fn test_rawtx_bincode_within_limit() {
        // Create a transaction within the size limit
        let data = vec![1u8; 1024]; // 1KB
        let tx = RawTx::new(data.clone());
        
        // Serialize with bincode
        let serialized = bincode::serialize(&tx).unwrap();
        
        // Deserialize should succeed
        let deserialized: RawTx = bincode::deserialize(&serialized).unwrap();
        assert_eq!(deserialized.data, data);
    }

    #[test]
    fn test_rawtx_bincode_exceeds_limit() {
        // Create a transaction that exceeds the size limit
        let data = vec![1u8; MAX_RAW_TX_SIZE + 1];
        let tx = RawTx::new(data);
        
        // Serialize with bincode
        let serialized = bincode::serialize(&tx).unwrap();
        
        // Deserialize should fail
        let result: Result<RawTx, _> = bincode::deserialize(&serialized);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.to_string().contains("exceeds maximum allowed size"));
    }

    #[test]
    fn test_rawtx_exactly_at_limit() {
        // Create a transaction exactly at the size limit
        let data = vec![1u8; MAX_RAW_TX_SIZE];
        let tx = RawTx::new(data.clone());
        
        // Test borsh
        let borsh_serialized = borsh::to_vec(&tx).unwrap();
        let borsh_result: RawTx = borsh::from_slice(&borsh_serialized).unwrap();
        assert_eq!(borsh_result.data.len(), MAX_RAW_TX_SIZE);
        
        // Test serde_json
        let json_serialized = serde_json::to_string(&tx).unwrap();
        let json_result: RawTx = serde_json::from_str(&json_serialized).unwrap();
        assert_eq!(json_result.data.len(), MAX_RAW_TX_SIZE);
        
        // Test bincode
        let bincode_serialized = bincode::serialize(&tx).unwrap();
        let bincode_result: RawTx = bincode::deserialize(&bincode_serialized).unwrap();
        assert_eq!(bincode_result.data.len(), MAX_RAW_TX_SIZE);
    }

    #[test]
    fn test_rawtx_empty() {
        // Test empty transaction
        let tx = RawTx::new(vec![]);
        
        // Test borsh
        let borsh_serialized = borsh::to_vec(&tx).unwrap();
        let borsh_result: RawTx = borsh::from_slice(&borsh_serialized).unwrap();
        assert!(borsh_result.is_empty());
        
        // Test serde_json
        let json_serialized = serde_json::to_string(&tx).unwrap();
        let json_result: RawTx = serde_json::from_str(&json_serialized).unwrap();
        assert!(json_result.is_empty());
        
        // Test bincode
        let bincode_serialized = bincode::serialize(&tx).unwrap();
        let bincode_result: RawTx = bincode::deserialize(&bincode_serialized).unwrap();
        assert!(bincode_result.is_empty());
    }
}
