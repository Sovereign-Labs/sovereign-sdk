use std::fmt::{Display, Formatter, Result};

use avail_rust_core::Error;

#[derive(Debug)]
pub struct AvailError(pub Error);

impl Display for AvailError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{:?}", self.0)
    }
}

impl From<Error> for AvailError {
    fn from(e: Error) -> Self {
        AvailError(e)
    }
}
