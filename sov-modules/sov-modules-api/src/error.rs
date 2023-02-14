use std::convert::Infallible;
use std::fmt::Debug;

/// Any kind of error during value decoding.
#[derive(Debug)]
pub struct DecodingError {}

pub enum DispatchError {
    Module(ModuleError),
}

impl Debug for DispatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Module(e) => f.debug_tuple("Module").field(&e.err).finish(),
        }
    }
}

pub struct ModuleError {
    pub err: String,
}

impl From<ModuleError> for DispatchError {
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
