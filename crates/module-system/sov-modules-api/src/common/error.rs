//! Module error definitions.

use std::fmt::{Debug, Display};
use std::sync::Arc;

use crate::prelude::serde::de::Error;
use serde::{ser::Error as _, Deserialize, Serialize};

/// Macro for creating structured error detail objects for the `ErrorDetail` trait.
///
/// This macro provides a convenient way to create `ErrorContext` objects (JSON objects)
/// that contain structured error information. It accepts the same syntax as `serde_json::json!`
/// but ensures the result is always a JSON object, which is required by the `ErrorDetail` trait.
///
/// The macro is particularly useful in `ErrorDetail` implementations where you need to
/// provide rich, structured error information to client applications.
///
/// # Panics
/// Panics if the provided JSON does not serialize to a JSON object (e.g., if you pass
/// a primitive value, array, or null instead of an object with key-value pairs).
///
/// # Examples
/// ```
/// use sov_modules_api::err_detail;
///
/// // Create error details for insufficient balance
/// let details = err_detail!({
///     "error_code": "insufficient_balance",
///     "amount": "100",
///     "balance": "50",
///     "token_id": "token123"
/// });
///
/// // Create error details with dynamic values
/// let amount = 100u64;
/// let balance = 50u64;
/// let details = err_detail!({
///     "error_code": "insufficient_balance",
///     "amount": amount.to_string(),
///     "balance": balance.to_string()
/// });
/// ```
#[macro_export]
macro_rules! err_detail {
    ($($json:tt)+) => {
        $crate::to_json_object(::serde_json::json!($($json)+))
    };
}

/// Calls [`serde_json::to_value`] on the given value but panics if the
/// resulting value is not a JSON object.
pub fn to_json_object<T: Serialize>(value: T) -> serde_json::Map<String, serde_json::Value> {
    let value = serde_json::to_value(value).unwrap();
    match value {
        serde_json::Value::Object(obj) => obj,
        _ => panic!("Expected serialization to produce a JSON object; got {value:?}"),
    }
}

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
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(err_detail!(self))
    }
}

/// Type alias for structured error detail objects.
///
/// This represents the standardized format for error details returned by the `ErrorDetail` trait.
/// It's a JSON object (map) containing structured information about error conditions, including
/// error codes, parameters, and contextual data that client-side applications can parse and
/// handle appropriately.
///
/// # Examples
/// ```json
/// {
///   "error_code": "insufficient_balance",
///   "amount": "100",
///   "balance": "50",
///   "from": "addr1",
///   "to": "addr2"
/// }
/// ```
/// Type alias for structured error detail objects.
///
/// This represents the standardized format for error details returned by the `ErrorDetail` trait.
/// It's a JSON object (map) containing structured information about error conditions, including
/// error codes, parameters, and contextual data that client-side applications can parse and
/// handle appropriately.
///
/// # Examples
/// ```json
/// {
///   "error_code": "insufficient_balance",
///   "amount": "100",
///   "balance": "50",
///   "from": "addr1",
///   "to": "addr2"
/// }
/// ```
pub type ErrorContext = serde_json::Map<String, serde_json::Value>;

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
/// ## Error Response Structure
///
/// When a module call fails, the structured error details returned by this trait are
/// automatically inserted into the `detail` field of the error response sent to clients.
/// This provides rich, machine-readable context about the failure beyond just the error message.
///
/// The trait requires `Debug + Display` to ensure all error types can be both printed for
/// debugging and formatted for user-facing messages, while the `error_detail` method provides
/// machine-readable structured data for client consumption.
pub trait ErrorDetail: Debug + Display {
    /// Returns detailed error information as a structured JSON object.
    ///
    /// This method should serialize the error into an `ErrorContext` (JSON object) containing
    /// all relevant information about the failure. The JSON structure should include error codes,
    /// context, and any parameters that caused the failure to enable proper error handling and debugging.
    ///
    /// **The returned JSON object will be automatically inserted into the `detail` field**
    /// of error responses when module calls fail, providing clients with structured error information.
    ///
    /// # Returns
    /// - `Ok(ErrorContext)` - Structured error details as a JSON object
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
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>>;
}

impl ErrorDetail for anyhow::Error {
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(err_detail!({ "message": format!("{self}") }))
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
#[derive(Clone, Debug)]
struct DeserializedError {
    data: ErrorContext,
}

impl ErrorDetail for DeserializedError {
    fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.data.clone())
    }
}

impl std::fmt::Display for DeserializedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            serde_json::to_string(&self.data)
                .unwrap_or_else(|e| format!("Error serializing error: {e}"))
        )
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
    /// - `Ok(ErrorContext)` - Structured error details from the wrapped error
    /// - `Err(Box<dyn Error>)` - If the underlying error detail extraction fails
    pub fn error_detail(&self) -> Result<ErrorContext, Box<dyn std::error::Error + Send + Sync>> {
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
        // Handle binary serialization
        if !serializer.is_human_readable() {
            let err = self.0.error_detail().map_err(S::Error::custom)?;
            let json = serde_json::to_string(&err).map_err(S::Error::custom)?;
            serializer.serialize_str(&json)
        } else {
            self.0
                .error_detail()
                .map_err(S::Error::custom)?
                .serialize(serializer)
        }
    }
}

impl<'de> Deserialize<'de> for ModuleError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if !deserializer.is_human_readable() {
            let json = String::deserialize(deserializer)?;
            let value: ErrorContext = serde_json::from_str(&json).map_err(D::Error::custom)?;
            Ok(Self(Arc::new(DeserializedError { data: value })))
        } else {
            let value: ErrorContext = Deserialize::deserialize(deserializer)?;
            Ok(Self(Arc::new(DeserializedError { data: value })))
        }
    }
}

/// Test that the module serializes identically (using JSON, which is what we serve) after a roundtrip through bincode.
#[test]
fn test_module_error_serialization_bincode() {
    let error = ModuleError::from(anyhow::anyhow!("test error"));
    let serialized = bincode::serialize(&error).unwrap();
    let deserialized: ModuleError = bincode::deserialize(&serialized).unwrap();
    assert_eq!(
        serde_json::to_string(&error).unwrap(),
        serde_json::to_string(&deserialized).unwrap()
    );
}

/// Test that the module serializes identically (using JSON, which is what we serve) after a roundtrip through serde_json.
#[test]
fn test_module_error_serialization_json() {
    let error = ModuleError::from(anyhow::anyhow!("test error"));
    let json = serde_json::to_string(&error).unwrap();
    let deserialized: ModuleError = serde_json::from_str(&json).unwrap();
    assert_eq!(json, serde_json::to_string(&deserialized).unwrap());
}
