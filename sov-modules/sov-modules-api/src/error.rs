use std::convert::Infallible;
use std::fmt::Debug;

/// Any kind of error during value decoding.
#[derive(Debug)]
pub struct DecodingError {}

/// General error type in the module system.
pub enum Error {
    /// Custom error thrown by a module.
    Module(ModuleError),
}

// We derive `Debug` by hand, because `ModuleError` doesn't implement `Debug` trait.
impl Debug for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Module(e) => f.debug_tuple("Module").field(&e.err).finish(),
        }
    }
}

/// Custom error thrown by a module.
// We can't derive `Debug` because it conflicts with blanket `From` implementation.
pub struct ModuleError {
    pub err: String,
}

impl From<ModuleError> for Error {
    fn from(err: ModuleError) -> Self {
        Self::Module(err)
    }
}

impl<T: Debug> From<T> for ModuleError {
    fn from(t: T) -> Self {
        Self {
            err: format!("{t:?}"),
        }
    }
}

impl From<Infallible> for DecodingError {
    fn from(_value: Infallible) -> Self {
        unreachable!()
    }
}
