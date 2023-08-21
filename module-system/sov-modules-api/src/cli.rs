use crate::CliWallet;

/// An argument to the cli containing a json string
#[derive(clap::Args, PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
pub struct JsonStringArg {
    /// The json formatted transaction data
    #[arg(long, help = "The JSON formatted transaction")]
    pub json: String,
}

/// An argument to the cli containing a path to a file
#[derive(clap::Args, PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
pub struct FileNameArg {
    /// The json formatted transaction data
    #[arg(long, help = "The JSON formatted transaction")]
    pub path: String,
}

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
