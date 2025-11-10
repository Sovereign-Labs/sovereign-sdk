//! Schema-based transaction serialization for Sovereign SDK Web3 interface.
//!
//! This module provides utilities for serializing transactions using schema-based
//! serialization, converting from JSON representations to efficient binary formats
//! using Borsh serialization. It supports both unsigned and signed transactions
//! with configurable schema validation.
//!
//! ## When to Use This Module
//!
//! Use this module when:
//! - Working with language bindings where Rust generics are not available
//! - Building dynamic systems that need runtime schema validation
//! - Creating Web3 interfaces for languages like JavaScript, Python, or Go
//! - The transaction types are not known at compile time
//! - You need to serialize transactions based on external schema definitions
//!
//! ## Comparison with `rust` Module
//!
//! This crate provides two different approaches for transaction building and serialization:
//!
//! ### `rust` Module (Compile-time Generics)
//! - **Use case**: Pure Rust applications with compile-time type safety
//! - **Type safety**: Full compile-time validation using Rust's type system
//! - **Performance**: Zero-cost abstractions with compile-time optimizations
//! - **Flexibility**: Requires specific generic type parameters (`Spec`, `ChainHash`, etc.)
//! - **Target audience**: Rust developers building native applications
//!
//! ### `schema` Module (Runtime Schema)
//! - **Use case**: Language bindings and dynamic systems
//! - **Type safety**: Runtime validation using JSON schema definitions
//! - **Performance**: Runtime serialization overhead but more flexible
//! - **Flexibility**: Works with any JSON-based transaction structure
//! - **Target audience**: Multi-language environments and dynamic systems
//!
//! The `schema` module is specifically designed for scenarios where Rust's
//! compile-time generics cannot be used, such as FFI bindings to other programming
//! languages or systems that need to work with dynamically defined transaction formats.

use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use sov_universal_wallet::schema::{RollupRoots, Schema};

pub use serde_json::json;

/// Errors that can occur during schema-based serialization operations.
#[derive(thiserror::Error, Debug)]
pub enum SerializerError {
    /// Error occurred during JSON to Borsh conversion using the schema.
    #[error("Borsh serialization error: {0}")]
    JsonToBorsh(#[source] sov_universal_wallet::schema::SchemaError),
    /// Error occurred during JSON serialization or deserialization.
    #[error("JSON serialization error: {0}")]
    JsonSerialization(#[from] serde_json::Error),
    /// The provided schema is invalid or malformed.
    #[error("Invalid schema: {0}")]
    InvalidSchema(#[source] serde_json::Error),
    #[error("Failed to calculate chain hash: {0}")]
    ChainHash(#[source] sov_universal_wallet::schema::SchemaError),
    /// Error occurred during HTTP request to fetch schema from URL.
    #[error("HTTP request error: {0}")]
    HttpRequest(#[from] reqwest::Error),
    #[error("schema object from http response was invalid")]
    InvalidSchemaResponse,
}

/// A schema-based serializer for converting transactions to binary format.
///
/// The `Serializer` uses a predefined schema to convert JSON-serialized transactions
/// into efficient Borsh binary format. This ensures consistent serialization across
/// different transaction types while maintaining compatibility with the rollup's
/// expected data formats.
pub struct Serializer {
    schema: Schema,
}

impl Serializer {
    /// Creates a new serializer with the provided schema.
    ///
    /// # Arguments
    ///
    /// * `schema` - The schema definition to use for serialization
    pub fn new(schema: Schema) -> Self {
        Self { schema }
    }

    /// Creates a serializer from a JSON schema string.
    ///
    /// Parses the provided JSON string into a schema and creates a new serializer
    /// instance with that schema.
    ///
    /// # Arguments
    ///
    /// * `s` - JSON string containing the schema definition
    ///
    /// # Returns
    ///
    /// Returns a `Serializer` instance or an error if the schema is invalid.
    ///
    /// # Errors
    ///
    /// * [`SerializerError::InvalidSchema`] - If the JSON string cannot be parsed as a valid schema
    pub fn from_json(s: &str) -> Result<Self, SerializerError> {
        let schema = Schema::from_json(s).map_err(SerializerError::InvalidSchema)?;
        Ok(Self::new(schema))
    }

    /// Creates a serializer by fetching a schema from a remote URL.
    ///
    /// This method will perform a synchronous HTTP GET request to the specified URL,
    /// retrieve the schema definition, and create a serializer instance.
    /// The response must be valid JSON that can be parsed as a schema.
    ///
    /// # Arguments
    ///
    /// * `url` - The URL to fetch the schema from
    ///
    /// # Returns
    ///
    /// Returns a `Serializer` instance or an error if the schema cannot be fetched or parsed.
    ///
    /// # Errors
    ///
    /// * [`SerializerError::HttpRequest`] - If the HTTP request fails
    /// * [`SerializerError::InvalidSchema`] - If the response cannot be parsed as a valid schema
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use sovereign_web3::schema::Serializer;
    ///
    /// fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let serializer = Serializer::from_url("https://example.com/schema.json")?;
    ///     Ok(())
    /// }
    /// ```
    pub fn from_url(url: &str) -> Result<Self, SerializerError> {
        let response = reqwest::blocking::get(url)?;
        let data = response.json::<serde_json::Map<String, serde_json::Value>>()?;
        let schema_json = data
            .get("schema")
            .ok_or(SerializerError::InvalidSchemaResponse)?;
        Self::from_json(&serde_json::to_string(schema_json)?)
    }

    /// Retrieves the 32-byte chain hash from the schema.
    ///
    /// The chain hash is a unique identifier for the blockchain network that helps
    /// prevent cross-chain replay attacks. This hash is embedded in the schema
    /// definition and must be concatenated to the unsigned transaction bytes when
    /// signing a transaction to ensure signatures are bound to a specific chain.
    ///
    /// # Returns
    ///
    /// Returns a 32-byte array representing the chain hash, or an error if the
    /// chain hash cannot be computed from the schema.
    ///
    /// # Errors
    ///
    /// * [`SerializerError::ChainHash`] - If the chain hash cannot be calculated from the schema
    pub fn chain_hash(&self) -> Result<[u8; 32], SerializerError> {
        Ok(self
            .schema
            .chain_hash()
            .map_err(SerializerError::ChainHash)?)
    }

    /// Serializes an unsigned transaction to binary format.
    ///
    /// Converts the provided unsigned transaction into a Borsh-serialized binary
    /// representation using the configured schema.
    ///
    /// # Note
    ///
    /// When signing an unsigned transaction, the chain hash must be appended, preferably
    /// use `UnsignedTransaction::bytes_for_signing` which handles this automatically.
    ///
    /// # Arguments
    ///
    /// * `unsigned_tx` - The unsigned transaction to serialize
    ///
    /// # Returns
    ///
    /// Returns the serialized bytes or an error if serialization fails.
    ///
    /// # Errors
    ///
    /// * [`SerializerError::JsonSerialization`] - If the transaction cannot be converted to JSON
    /// * [`SerializerError::JsonToBorsh`] - If the JSON cannot be converted to Borsh format
    pub fn serialize_unsigned_tx(
        &self,
        unsigned_tx: &UnsignedTransaction,
    ) -> Result<Vec<u8>, SerializerError> {
        self.serialize(unsigned_tx, RollupRoots::UnsignedTransaction)
    }

    /// Serializes a signed transaction to binary format.
    ///
    /// Converts the provided signed transaction into a Borsh-serialized binary
    /// representation using the configured schema.
    ///
    /// # Arguments
    ///
    /// * `tx` - The signed transaction to serialize
    ///
    /// # Returns
    ///
    /// Returns the serialized bytes or an error if serialization fails.
    ///
    /// # Errors
    ///
    /// * [`SerializerError::JsonSerialization`] - If the transaction cannot be converted to JSON
    /// * [`SerializerError::JsonToBorsh`] - If the JSON cannot be converted to Borsh format
    pub fn serialize_tx(&self, tx: &Transaction) -> Result<Vec<u8>, SerializerError> {
        self.serialize(tx, RollupRoots::Transaction)
    }

    /// Internal method to serialize any serializable type using the schema.
    ///
    /// This method handles the common serialization logic for both unsigned and signed transactions.
    /// It first converts the input to JSON, then uses the schema to convert the JSON to Borsh format.
    ///
    /// # Arguments
    ///
    /// * `input` - The object to serialize (must implement `Serialize`)
    /// * `root` - The schema root type to use for serialization
    ///
    /// # Returns
    ///
    /// Returns the serialized bytes or an error if serialization fails.
    fn serialize<T: Serialize>(
        &self,
        input: &T,
        root: RollupRoots,
    ) -> Result<Vec<u8>, SerializerError> {
        let json_str = serde_json::to_string(input)?;
        let root = self
            .schema
            .rollup_expected_index(root)
            .map_err(SerializerError::JsonToBorsh)?;
        let bytes = self
            .schema
            .json_to_borsh(root, &json_str)
            .map_err(SerializerError::JsonToBorsh)?;
        Ok(bytes)
    }
}

/// Default maximum priority fee in basis points (0).
pub const DEFAULT_MAX_PRIORITY_FEE_BIPS: u64 = 0;

/// Default maximum fee amount (100,000,000 units).
pub const DEFAULT_MAX_FEE: u128 = 100000000;

/// Generates default uniqueness data based on current timestamp.
///
/// Creates a `UniquenessData::Generation` variant using the current
/// Unix timestamp in seconds. This provides a simple way to ensure
/// transaction uniqueness without requiring explicit nonce management.
///
/// # Returns
///
/// Returns `UniquenessData::Generation` with the current Unix timestamp
///
/// # Panics
///
/// Panics if the system clock is set to a time before the Unix epoch.
pub fn default_uniqueness() -> Result<UniquenessData, TransactionBuilderError> {
    use std::time::{SystemTime, UNIX_EPOCH};
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    Ok(UniquenessData::Generation(now))
}

/// Errors that can occur when building transactions using the schema-based approach.
#[derive(thiserror::Error, Debug)]
pub enum TransactionBuilderError {
    /// The chain ID is required but was not provided in the transaction details.
    #[error("chain_id is a required field but was not provided")]
    MissingChainId,
    #[error("system time error: {0}")]
    TimeError(#[from] std::time::SystemTimeError),
}

/// Defines how transaction uniqueness is enforced to prevent replay attacks.
///
/// The uniqueness mechanism ensures that each transaction can only be executed once
/// on the blockchain. Two different strategies are supported: nonce-based and generation-based.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniquenessData {
    /// Nonce-based uniqueness using sequential account nonces.
    ///
    /// An account's transactions must have unique and consecutive nonces,
    /// similar to Ethereum's transaction ordering mechanism.
    Nonce(u64),
    /// Generation-based uniqueness using timestamp generations.
    ///
    /// The last `PAST_TRANSACTION_GENERATION` generations are cached.
    /// Transactions older than this buffer are invalid, transactions falling
    /// within it or with a higher generation are valid but must have a unique
    /// hash within their generation. This allows for more flexible transaction
    /// ordering while still preventing replays.
    Generation(u64),
}

/// Type alias for runtime calls represented as JSON objects.
pub type RuntimeCall = serde_json::Value;

/// Transaction execution details including fees, gas limits, and chain identification.
///
/// This structure contains the metadata required for transaction execution,
/// separate from the actual call data. It includes fee specifications, resource
/// limits, and chain identification information.
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TxDetails {
    /// Maximum priority fee in basis points (1/100th of a percent).
    ///
    /// This represents the additional fee willing to be paid to prioritize
    /// the transaction in the mempool.
    pub max_priority_fee_bips: u64,
    /// Maximum total fee willing to be paid for transaction execution.
    ///
    /// This represents the absolute ceiling for transaction costs, including
    /// both base fees and priority fees. Serialized as a string for JSON compatibility.
    #[serde_as(as = "serde_with::DisplayFromStr")]
    pub max_fee: u128,
    /// Optional gas limit for transaction execution.
    ///
    /// If `None`, the transaction has no gas limit. If `Some(vec)`, each element
    /// represents a gas limit for different execution phases or modules.
    pub gas_limit: Option<Vec<u64>>,
    /// Chain identifier to prevent cross-chain replay attacks.
    ///
    /// This ensures transactions can only be executed on the intended blockchain.
    pub chain_id: u64,
}

/// An unsigned transaction ready to be signed.
///
/// This structure represents a complete transaction that has been constructed
/// with all necessary parameters but has not yet been cryptographically signed.
/// It contains the call to be executed, uniqueness data to prevent replays,
/// and execution details such as fees and gas limits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct UnsignedTransaction {
    /// The runtime call to be executed when this transaction is processed.
    pub runtime_call: RuntimeCall,
    /// Uniqueness data to prevent transaction replay attacks.
    pub uniqueness: UniquenessData,
    /// Transaction execution details including fees and gas limits.
    pub details: TxDetails,
}

impl UnsignedTransaction {
    pub fn bytes_for_signing(&self, serializer: &Serializer) -> Result<Vec<u8>, SerializerError> {
        let mut bytes = serializer.serialize_unsigned_tx(self)?;
        let chain_hash = serializer.chain_hash()?;
        bytes.extend_from_slice(&chain_hash);
        Ok(bytes)
    }

    pub fn to_signed(&self, pub_key: Vec<u8>, signature: Vec<u8>) -> Transaction {
        Transaction::V0(TransactionV0 {
            pub_key,
            signature,
            runtime_call: self.runtime_call.clone(),
            uniqueness: self.uniqueness,
            details: self.details.clone(),
        })
    }
}

/// Version 0 of a signed transaction format.
///
/// This structure represents a fully signed transaction that can be submitted
/// to the blockchain for execution. It includes the cryptographic signature
/// and public key along with all the transaction data from the unsigned version.
#[serde_as]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TransactionV0 {
    /// The public key corresponding to the private key used to sign this transaction.
    #[serde_as(as = "serde_with::hex::Hex")]
    pub pub_key: Vec<u8>,
    /// The cryptographic signature proving authorization for this transaction.
    #[serde_as(as = "serde_with::hex::Hex")]
    pub signature: Vec<u8>,
    /// The runtime call to be executed when this transaction is processed.
    pub runtime_call: RuntimeCall,
    /// Uniqueness data to prevent transaction replay attacks.
    pub uniqueness: UniquenessData,
    /// Transaction execution details including fees and gas limits.
    pub details: TxDetails,
}

/// A versioned transaction envelope supporting different transaction formats.
///
/// This enum allows for forward compatibility by supporting multiple transaction
/// versions. As the protocol evolves, new transaction formats can be added
/// while maintaining backward compatibility with existing formats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transaction {
    /// Version 0 transaction format.
    V0(TransactionV0),
}

/// A builder for constructing unsigned transactions with customizable parameters.
///
/// The `TransactionBuilder` provides a fluent interface for building transactions
/// with various optional parameters like fees, gas limits, and uniqueness data.
/// Once configured, it produces an unsigned transaction that can be signed later.
/// Unlike the generic `TransactionBuilder` in the rust module, this version works
/// with the JSON-based schema serialization approach.
///
/// # Examples
///
/// ```ignore
/// let builder = TransactionBuilder::new(my_call)
///     .max_fee(1000u128)
///     .priority_fee_bips(100u64)
///     .chain_id(1)
///     .uniqueness(my_uniqueness_data);
///
/// let unsigned_tx = builder.build()?;
/// ```
pub struct TransactionBuilder {
    call: RuntimeCall,
    uniqueness: Option<UniquenessData>,
    priority_fee_bips: Option<u64>,
    max_fee: Option<u128>,
    gas_limit: Option<Option<Vec<u64>>>,
    chain_id: Option<u64>,
}

impl TransactionBuilder {
    /// Creates a new transaction builder with the specified runtime call.
    ///
    /// All optional parameters are initially unset and will use default values
    /// when the transaction is built. The chain_id is required and must be set
    /// before calling `build()`.
    ///
    /// # Arguments
    ///
    /// * `call` - The runtime call to include in the transaction
    pub fn new(call: RuntimeCall) -> Self {
        Self {
            call,
            uniqueness: None,
            priority_fee_bips: None,
            max_fee: None,
            gas_limit: None,
            chain_id: None,
        }
    }

    /// Sets the priority fee in basis points.
    ///
    /// If not set, defaults to [`DEFAULT_MAX_PRIORITY_FEE_BIPS`].
    ///
    /// # Arguments
    ///
    /// * `priority_fee_bips` - The priority fee amount in basis points
    pub fn priority_fee_bips(mut self, priority_fee_bips: u64) -> Self {
        self.priority_fee_bips = Some(priority_fee_bips);
        self
    }

    /// Sets the maximum fee willing to be paid for the transaction.
    ///
    /// This represents the total maximum amount that can be charged for executing
    /// the transaction. If not set, defaults to [`DEFAULT_MAX_FEE`].
    ///
    /// # Arguments
    ///
    /// * `max_fee` - The maximum fee amount
    pub fn max_fee(mut self, max_fee: u128) -> Self {
        self.max_fee = Some(max_fee);
        self
    }

    /// Sets the uniqueness data for the transaction.
    ///
    /// Uniqueness data helps prevent transaction replay attacks by ensuring
    /// each transaction is unique. If not set, defaults to timestamp-based
    /// uniqueness via [`default_uniqueness`].
    ///
    /// # Arguments
    ///
    /// * `uniqueness` - The uniqueness data to use
    pub fn uniqueness(mut self, uniqueness: UniquenessData) -> Self {
        self.uniqueness = Some(uniqueness);
        self
    }

    /// Sets the gas limit for the transaction.
    ///
    /// The gas limit defines the maximum amount of computational work
    /// the transaction is allowed to perform. Pass `None` for unlimited gas,
    /// or `Some(vec)` where each element represents a limit for different
    /// execution phases or modules.
    ///
    /// # Arguments
    ///
    /// * `gas_limit` - The gas limit (None for unlimited, Some(vec) for specific limits)
    pub fn gas_limit(mut self, gas_limit: Option<Vec<u64>>) -> Self {
        self.gas_limit = Some(gas_limit);
        self
    }

    /// Sets the chain ID for the transaction.
    ///
    /// The chain ID is required and must be set before calling `build()`.
    /// It prevents transactions from being replayed on different chains.
    ///
    /// # Arguments
    ///
    /// * `chain_id` - The chain identifier
    pub fn chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = Some(chain_id);
        self
    }

    /// Builds an unsigned transaction with the configured parameters.
    ///
    /// Uses default values for any parameters that were not explicitly set:
    /// - Priority fee: [`DEFAULT_MAX_PRIORITY_FEE_BIPS`]
    /// - Max fee: [`DEFAULT_MAX_FEE`]
    /// - Gas limit: `None` (unlimited)
    /// - Uniqueness: Generated via [`default_uniqueness`]
    ///
    /// The chain_id must be set explicitly before calling this method.
    ///
    /// # Returns
    ///
    /// Returns an `UnsignedTransaction` that can be signed later, or an error
    /// if the transaction could not be constructed.
    ///
    /// # Errors
    ///
    /// * [`TransactionBuilderError::MissingChainId`] - If chain_id was not set
    pub fn build(self) -> Result<UnsignedTransaction, TransactionBuilderError> {
        let priority_fee = self
            .priority_fee_bips
            .unwrap_or(DEFAULT_MAX_PRIORITY_FEE_BIPS);
        let max_fee = self.max_fee.unwrap_or(DEFAULT_MAX_FEE);
        let gas_limit = self.gas_limit.unwrap_or(None);
        let uniqueness = match self.uniqueness {
            Some(u) => u,
            None => default_uniqueness()?,
        };
        let chain_id = self
            .chain_id
            .ok_or(TransactionBuilderError::MissingChainId)?;

        Ok(UnsignedTransaction {
            runtime_call: self.call,
            uniqueness,
            details: TxDetails {
                max_priority_fee_bips: priority_fee,
                max_fee,
                gas_limit,
                chain_id,
            },
        })
    }
}
