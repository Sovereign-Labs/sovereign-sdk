use sov_rollup_interface::maybestd::string::String;

use crate::CliWallet;

/// An argument to the cli containing a json string
#[derive(PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
#[cfg_attr(feature = "dep:clap", clap::Args)]
pub struct JsonStringArg {
    /// The json formatted transaction data
    #[cfg_attr(
        feature = "dep:clap",
        arg(long, help = "The JSON formatted transaction")
    )]
    pub json: String,
}

/// An argument to the cli containing a path to a file
#[derive(PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
#[cfg_attr(feature = "dep:clap", clap::Args)]
pub struct FileNameArg {
    /// The json formatted transaction data
    #[cfg_attr(
        feature = "dep:clap",
        arg(long, help = "The JSON formatted transaction")
    )]
    pub path: String,
}

#[cfg(feature = "std")]
impl TryFrom<FileNameArg> for JsonStringArg {
    type Error = std::io::Error;
    fn try_from(arg: FileNameArg) -> Result<Self, Self::Error> {
        let json = std::fs::read_to_string(arg.path)?;
        Ok(JsonStringArg { json })
    }
}

pub trait CliFrontEnd<RT>
where
    RT: CliWallet,
{
    type CliIntermediateRepr<U>;
}
