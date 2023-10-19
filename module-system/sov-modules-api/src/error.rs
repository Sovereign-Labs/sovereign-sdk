/// General error type in the Module System.
#[derive(Debug)]
#[cfg_attr(feature = "std", derive(thiserror::Error))]
pub enum Error {
    /// Custom error thrown by a module.
    #[cfg_attr(feature = "std", error(transparent))]
    ModuleError(#[cfg_attr(feature = "std", from)] anyhow::Error),
}

#[cfg(not(feature = "std"))]
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[cfg(not(feature = "std"))]
impl From<Error> for anyhow::Error {
    fn from(e: Error) -> Self {
        anyhow::Error::msg(e)
    }
}

#[cfg(not(feature = "std"))]
impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Self::ModuleError(e)
    }
}
