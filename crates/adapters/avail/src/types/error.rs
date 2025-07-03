use std::fmt::{Display, Formatter, Result};

use avail_rust::error::ClientError;

#[derive(Debug)]
pub struct AvailError(pub ClientError);

impl Display for AvailError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "{}", self.0.to_string())
    }
}
