use std::fs;

use serde::de::DeserializeOwned;
use sov_modules_core::DispatchCall;

/// An argument definition for a sov-cli application that decodes into a runtime call message.
///
/// Typically, this is a string representation of common formats, such as JSON.
pub trait CliWalletArg<RT: DispatchCall> {
    /// The decoding error representation.
    type Error;

    /// Decoded the instance into a runtime call message.
    fn decode_call_from_readable(self) -> Result<RT::Decodable, Self::Error>;
}

/// An argument to the cli containing a json string
#[derive(clap::Args, PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
pub struct JsonStringArg {
    /// The json formatted transaction data
    #[arg(long, help = "The JSON formatted transaction")]
    pub json: String,
}

/// An argument to the cli containing a path to a file
#[derive(clap::Args, PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
pub struct JsonFileNameArg {
    /// The json formatted transaction data
    #[arg(long, help = "The JSON formatted transaction")]
    pub path: String,
}

impl<RT> CliWalletArg<RT> for JsonStringArg
where
    RT: DispatchCall,
    RT::Decodable: DeserializeOwned,
{
    type Error = anyhow::Error;

    fn decode_call_from_readable(self) -> anyhow::Result<RT::Decodable> {
        Ok(serde_json::from_str(&self.json)?)
    }
}

impl<RT> CliWalletArg<RT> for JsonFileNameArg
where
    RT: DispatchCall,
    RT::Decodable: DeserializeOwned,
{
    type Error = anyhow::Error;

    fn decode_call_from_readable(self) -> anyhow::Result<RT::Decodable> {
        let contents = fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&contents)?)
    }
}
