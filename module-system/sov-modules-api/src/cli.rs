use std::fs;

use crate::{clap, CliWallet};

pub trait CliFrontEnd<RT>
where
    RT: CliWallet,
{
    type CliIntermediateRepr<U>;
}

pub trait CliTxImportArg {
    /// The chain ID of the transaction.
    fn chain_id(&self) -> u64;

    /// The gas tip for the sequencer.
    fn gas_tip(&self) -> u64;

    /// The gas limit for the transaction execution.
    fn gas_limit(&self) -> u64;
}

/// An argument to the cli containing a json string
#[derive(clap::Args, PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
pub struct JsonStringArg {
    /// The json formatted transaction data
    #[arg(long, help = "The JSON formatted transaction")]
    pub json: String,

    /// The chain ID of the transaction.
    #[arg(long, help = "The chain ID of the transaction.")]
    pub chain_id: u64,

    /// The gas tip for the sequencer.
    #[arg(long, help = "The gas tip for the sequencer.", default_value = "0")]
    pub gas_tip: u64,

    /// The gas limit for the transaction execution.
    #[arg(
        long,
        help = "The gas limit for the transaction execution.",
        default_value = "0"
    )]
    pub gas_limit: u64,
}

/// An argument to the cli containing a path to a file
#[derive(clap::Args, PartialEq, core::fmt::Debug, Clone, PartialOrd, Ord, Eq, Hash)]
pub struct FileNameArg {
    /// The json formatted transaction data
    #[arg(long, help = "The JSON formatted transaction")]
    pub path: String,

    /// The chain ID of the transaction.
    #[arg(long, help = "The chain ID of the transaction.")]
    pub chain_id: u64,

    /// The gas tip for the sequencer.
    #[arg(long, help = "The gas tip for the sequencer.", default_value = "0")]
    pub gas_tip: u64,

    /// The gas limit for the transaction execution.
    #[arg(
        long,
        help = "The gas limit for the transaction execution.",
        default_value = "0"
    )]
    pub gas_limit: u64,
}

impl CliTxImportArg for JsonStringArg {
    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    fn gas_tip(&self) -> u64 {
        self.gas_tip
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }
}

impl CliTxImportArg for FileNameArg {
    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    fn gas_tip(&self) -> u64 {
        self.gas_tip
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }
}

impl TryFrom<FileNameArg> for JsonStringArg {
    type Error = std::io::Error;
    fn try_from(arg: FileNameArg) -> Result<Self, Self::Error> {
        let FileNameArg {
            path,
            chain_id,
            gas_tip,
            gas_limit,
        } = arg;

        Ok(JsonStringArg {
            json: fs::read_to_string(path)?,
            chain_id,
            gas_tip,
            gas_limit,
        })
    }
}
