//! Module error definitions.

use std::fmt::{Debug, Display};
use std::sync::Arc;

use serde::{ser::Error as _, Deserialize, Serialize};

/// A bech32 address parse error.
#[derive(Debug, thiserror::Error)]
pub enum Bech32ParseError {
    /// Bech32 decoding error represented via [`bech32::primitives::decode::CheckedHrpstringError`].
    #[error("Bech32 error: {0}")]
    Bech32(#[from] bech32::primitives::decode::CheckedHrpstringError),
    /// The provided "Human-Readable Part" is invalid.
    #[error("Wrong HRP: {0}")]
    WrongHRP(String),
    /// The provided address length is invalid.
    #[error("Wrong address length. Expected: {0} bytes, got: {1}")]
    WrongLength(usize, usize),
}

/// Core error types that can occur during module operations.
///
/// This enum provides a unified error interface for fundamental module system operations,
/// particularly around state access and generic error handling. All variants are transparent
/// to preserve the original error information while providing structured error categorization.
#[derive(Debug, thiserror::Error)]
pub enum CoreModuleError {
    /// A generic error that doesn't fit into specific categories.
    ///
    /// This variant wraps arbitrary errors using `anyhow::Error`, providing a catch-all
    /// for unexpected errors or errors from external dependencies that don't have
    /// specific handling in the module system.
    #[error(transparent)]
    Generic(#[from] anyhow::Error),
    /// An error occurred while reading from the module's state.
    ///
    /// This variant captures errors that occur during state read operations, such as
    /// accessing stored values, iterating over state entries, or deserializing state data.
    #[error(transparent)]
    StateRead(Box<dyn std::error::Error + Send + Sync>),
    /// An error occurred while writing to the module's state.
    ///
    /// This variant captures errors that occur during state write operations, such as
    /// setting values, deleting entries, or serializing data for storage. Common causes
    /// include gas limit exceeded during writes.
    #[error(transparent)]
    StateWrite(Box<dyn std::error::Error + Send + Sync>),
}

impl CoreModuleError {
    /// Creates a new `StateRead` error variant from any error type.
    ///
    /// This constructor function provides a convenient way to wrap state read errors
    /// into the standardized `CoreModuleError::StateRead` variant.
    ///
    /// The `E` type parameter follows the same trait bounds used on state accessors.
    pub fn state_read<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        Self::StateRead(Box::new(err))
    }

    /// Creates a new `StateWrite` error variant from any error type.
    ///
    /// This constructor function provides a convenient way to wrap state write errors
    /// into the standardized `CoreModuleError::StateWrite` variant.
    ///
    /// The `E` type parameter follows the same trait bounds used on state accessors.
    pub fn state_write<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        Self::StateWrite(Box::new(err))
    }
}

/// Errors are serializable in order to provide structured error information for the [`ErrorDetail`] trait.
/// [`anyhow::Error`] & State errors don't implement `Serialize` so we have to do it manually here.
impl serde::Serialize for CoreModuleError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let mut state = serializer.serialize_struct("CoreModuleError", 2)?;

        match self {
            CoreModuleError::Generic(e) => {
                state.serialize_field("error_code", "generic")?;
                state.serialize_field("message", &e.to_string())?;
            }
            CoreModuleError::StateRead(e) => {
                state.serialize_field("error_code", "state_read")?;
                state.serialize_field("message", &e.to_string())?;
            }
            CoreModuleError::StateWrite(e) => {
                state.serialize_field("error_code", "state_write")?;
                state.serialize_field("message", &e.to_string())?;
            }
        }

        state.end()
    }
}

impl ErrorDetail for CoreModuleError {
    fn error_detail(&self) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok(serde_json::to_value(self).expect("Serialization of CoreModuleError should not fail"))
    }
}

/// Trait for providing structured error details as JSON for client-side applications.
///
/// This trait defines how module errors should be returned to client-side applications by
/// providing rich, structured information about failure conditions that can be serialized
/// and transmitted over APIs. It's the primary mechanism for exposing module errors through
/// rollup simulation endpoints and error reporting systems.
///
/// **Module implementations should implement this trait on their `Error` associated type**
/// to provide useful structured data to client-side applications, enabling proper error
/// handling, user feedback, and debugging capabilities.
///
/// The trait requires `Debug + Display` to ensure all error types can be both printed for
/// debugging and formatted for user-facing messages, while the `error_detail` method provides
/// machine-readable structured data for client consumption.
pub trait ErrorDetail: Debug + Display {
    /// Returns detailed error information as a structured JSON value.
    ///
    /// This method should serialize the error into a `serde_json::Value` containing all relevant
    /// information about the failure. The JSON structure can include error codes, context,
    /// and any parameters that caused the failure to enable proper error handling and debugging.
    ///
    /// # Returns
    /// - `Ok(serde_json::Value)` - Structured error details in JSON format
    /// - `Err(Box<dyn Error>)` - If serialization fails (though implementations should avoid this)
    ///
    /// # Examples
    /// Typical JSON structure might include:
    /// ```json
    /// {
    ///   "error_code": "insufficient_balance",
    ///   "amount": "100",
    ///   "balance": "50",
    ///   "from": "addr1",
    ///   "to": "addr2"
    /// }
    /// ```
    fn error_detail(&self) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>;
}

impl ErrorDetail for anyhow::Error {
    fn error_detail(&self) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok(serde_json::json!({ "message": format!("{self}") }))
    }
}

/// A type-erased error representation used for deserializing `ModuleError`.
///
/// Since `ModuleError` wraps `dyn ErrorDetail` trait objects, it cannot directly deserialize
/// back to the original concrete error type. This struct serves as a type-less container
/// that holds the raw JSON error data, allowing `ModuleError` to implement `Deserialize`
/// by wrapping any deserialized error data in this generic representation.
///
/// When `ModuleError` is deserialized, the JSON data is captured in this struct, which
/// then implements `ErrorDetail` by simply returning the stored JSON data unchanged.
#[derive(Clone, Debug, derive_more::Display)]
struct DeserializedError {
    data: serde_json::Value,
}

impl ErrorDetail for DeserializedError {
    fn error_detail(&self) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.data.clone())
    }
}

/// A type-erased module error that implements `ErrorDetail`.
///
/// This struct wraps any error type that implements `ErrorDetail` in a thread-safe,
/// cloneable wrapper. It's used throughout the module system to provide a uniform
/// error interface while preserving the detailed error information from specific
/// module implementations.
///
/// The wrapper uses `Arc` for efficient cloning and to satisfy the `Send + Sync`
/// requirements for use across thread boundaries in the rollup system.
#[derive(Debug, Clone, derive_more::Display)]
pub struct ModuleError(Arc<dyn ErrorDetail + Send + Sync>);

impl ModuleError {
    /// Extracts detailed error information as structured JSON.
    ///
    /// This method delegates to the underlying error's `error_detail` implementation
    /// to provide rich, structured information about the failure condition. The returned
    /// JSON can be used for API responses, logging, and debugging.
    ///
    /// # Returns
    /// - `Ok(serde_json::Value)` - Structured error details from the wrapped error
    /// - `Err(Box<dyn Error>)` - If the underlying error detail extraction fails
    pub fn error_detail(
        &self,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        self.0.error_detail()
    }
}

impl PartialEq for ModuleError {
    fn eq(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl Eq for ModuleError {}

impl<T: ErrorDetail + Send + Sync + 'static> From<T> for ModuleError {
    fn from(value: T) -> Self {
        Self(Arc::new(value))
    }
}

impl Serialize for ModuleError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0
            .error_detail()
            .map_err(S::Error::custom)?
            .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ModuleError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(Self(Arc::new(DeserializedError { data: value })))
    }
}
