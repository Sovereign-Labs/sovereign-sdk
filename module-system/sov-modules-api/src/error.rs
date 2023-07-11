use std::fmt::Debug;

use thiserror::Error;

/// General error type in the Module System.
#[derive(Debug, Error)]
pub enum Error {
    /// Custom error thrown by a module.
    #[error(transparent)]
    ModuleError(#[from] anyhow::Error),
}
