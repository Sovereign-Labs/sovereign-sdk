//! Module error definitions.

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

/// General error type in the Module System.
#[derive(Debug, thiserror::Error)]
pub enum ModuleError {
    /// Custom error thrown by a module.
    #[error(transparent)]
    ModuleError(#[from] anyhow::Error),
}

impl serde::Serialize for ModuleError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let error = match self {
            ModuleError::ModuleError(e) => format!("{e:}"),
        };
        error.serialize(serializer)
    }
}

impl<'de> serde::Deserialize<'de> for ModuleError {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let error = String::deserialize(deserializer)?;
        Ok(Self::ModuleError(anyhow::Error::msg(error)))
    }
}

/// We are manually implementing clone because the inner [`anyhow::Error`] doesn't implement clone.
/// We need to manually loop through the error chain to not loose any of the error context. The intermediate
/// error types are not clonable so we need to manually convert them to strings.
impl Clone for ModuleError {
    fn clone(&self) -> Self {
        match self {
            Self::ModuleError(anyhow_err) => {
                let mut chain = anyhow_err.chain();

                Self::ModuleError(if let Some(err) = chain.next() {
                    let mut output = anyhow::Error::msg(err.to_string());

                    for outer_err in chain {
                        output = output.context(anyhow::Error::msg(outer_err.to_string()));
                    }

                    output
                } else {
                    anyhow::anyhow!("Empty error message")
                })
            }
        }
    }
}

impl PartialEq for ModuleError {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::ModuleError(e1), Self::ModuleError(e2)) => e1.to_string() == e2.to_string(),
        }
    }
}

impl Eq for ModuleError {}

#[cfg(test)]
mod test {
    use anyhow::anyhow;

    use crate::ModuleError;

    #[test]
    fn test_module_error_roundtrip() {
        let error = ModuleError::ModuleError(anyhow::Error::msg("test"));
        let serialized = serde_json::to_string(&error).unwrap();
        let deserialized: ModuleError = serde_json::from_str(&serialized).unwrap();
        // We can only asserts start, because RUST_BACKTRACE can alter full output
        assert!(deserialized.to_string().starts_with("test"));
    }

    /// Tests that the inner error context gets correctly propagated when copying an error.
    #[test]
    fn test_module_error_copy() {
        let error = anyhow!("Inner message").context("Outer context".to_string());

        let cloned_err = ModuleError::ModuleError(error).clone();

        match cloned_err {
            ModuleError::ModuleError(cloned_err) => {
                let mut chained_clone = cloned_err.chain();

                assert_eq!(
                    chained_clone.len(),
                    2,
                    "The cloned error doesn't have the correct length"
                );
                assert_eq!(
                    chained_clone.next().unwrap().to_string(),
                    "Inner message",
                    "The inner message has not been correctly cloned"
                );
                assert_eq!(
                    chained_clone.next().unwrap().to_string(),
                    "Outer context",
                    "The outer context has not been correctly cloned"
                );
            }
        }
    }
}
