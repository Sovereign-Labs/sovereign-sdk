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

/// todo
#[derive(Debug, thiserror::Error)]
pub enum CoreModuleError {
    /// todo
    #[error(transparent)]
    Generic(#[from] anyhow::Error),
    ///todo
    #[error(transparent)]
    StateRead(Box<dyn std::error::Error + Send + Sync>),
    /// TODO
    #[error(transparent)]
    StateWrite(Box<dyn std::error::Error + Send + Sync>),
}

impl CoreModuleError {
    /// todo
    pub fn state_read<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        Self::StateRead(Box::new(err))
    }

    /// TODO
    pub fn state_write<E: std::error::Error + Send + Sync + 'static>(err: E) -> Self {
        Self::StateWrite(Box::new(err))
    }
}

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

/// TODO
pub trait ErrorDetail: Debug + Display {
    /// TODO
    fn error_detail(&self) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>>;
}

impl ErrorDetail for anyhow::Error {
    fn error_detail(&self) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok(serde_json::json!({ "message": format!("{self}") }))
    }
}

#[derive(Clone, Debug, derive_more::Display)]
struct DeserializedError {
    data: serde_json::Value,
}

impl ErrorDetail for DeserializedError {
    fn error_detail(&self) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
        Ok(self.data.clone())
    }
}

/// TODO
#[derive(Debug, Clone, derive_more::Display)]
pub struct ModuleError(Arc<dyn ErrorDetail + Send + Sync>);

impl ModuleError {
    /// TODO
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
